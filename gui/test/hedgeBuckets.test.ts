/**
 * hedgeBuckets — the pure logic behind the Hedge Flow dashboard: which buckets need
 * hedging (vessel fill, worst-first ordering) and where fired risk actually went
 * (crossed / hedged / warehoused).
 */

import { describe, expect, it } from "vitest";

import type { HedgeIntent, HedgeProvenance } from "../src/data/contract";
import {
  bandRank,
  bookLevelFires,
  bucketsFromIntents,
  flowShares,
  flowTotals,
  hedgesForPosition,
  latestIntentByBook,
} from "../src/lib/hedgeBuckets";

/** A live intent tick. Only the fields the bucket board reads are meaningful. */
function intent(
  book: string,
  band: string,
  utilization: number,
  netRisk = -1000,
  threshold = 5000,
): HedgeIntent {
  return {
    book,
    instrument: "US91282CJL63",
    action: null,
    band,
    netRisk,
    threshold,
    utilization,
    overflow: 0,
    size: 0,
  } as HedgeIntent;
}

/** A fired hedge record. */
function fire(
  crossed: number,
  hedged: number,
  residual: number,
  opts: { advisory?: boolean; parent?: bigint } = {},
): HedgeProvenance {
  return {
    hedgeId: `HDG-${crossed}-${hedged}`,
    book: "rates-usd",
    instrument: "US91282CJL63",
    internalCrossed: crossed,
    externalHedged: hedged,
    residual,
    advisory: opts.advisory ?? false,
    parentPositionId: opts.parent,
    band: "red",
    utilization: 0.9,
  } as HedgeProvenance;
}

describe("hedgeBuckets — the bucket board", () => {
  it("collapses a repeating intent stream to the latest state per book", () => {
    const stream = [
      intent("rates-usd", "green", 0.2),
      intent("wash-book", "amber", 0.7),
      intent("rates-usd", "red", 0.94),
    ];
    const latest = latestIntentByBook(stream);
    expect(latest).toHaveLength(2);
    expect(latest.find((i) => i.book === "rates-usd")?.utilization).toBe(0.94);
  });

  it("orders worst-first: breach, red, amber, green", () => {
    const buckets = bucketsFromIntents([
      intent("a-green", "green", 0.3),
      intent("b-breach", "breach", 1.27),
      intent("c-amber", "amber", 0.68),
      intent("d-red", "red", 0.94),
    ]);
    expect(buckets.map((b) => b.book)).toEqual([
      "b-breach",
      "d-red",
      "c-amber",
      "a-green",
    ]);
  });

  it("is STABLE for equal band and utilisation, so rows do not swap on a tick", () => {
    const buckets = bucketsFromIntents([
      intent("zulu", "amber", 0.5),
      intent("alpha", "amber", 0.5),
    ]);
    expect(buckets.map((b) => b.book)).toEqual(["alpha", "zulu"]);
  });

  it("saturates the drawn fill at the brim but reports the overflow honestly", () => {
    const [over] = bucketsFromIntents([intent("rates-usd", "breach", 1.27)]);
    expect(over?.fill).toBe(1); // the vessel cannot draw past full…
    expect(over?.overflow).toBeCloseTo(0.27, 10); // …but the excess is not lost
    expect(over?.utilization).toBeCloseTo(1.27, 10);
    expect(over?.needsHedge).toBe(true);
  });

  it("flags a bucket exactly at its threshold as needing a hedge", () => {
    const [at] = bucketsFromIntents([intent("rates-usd", "red", 1)]);
    expect(at?.needsHedge).toBe(true);
    expect(at?.overflow).toBe(0);
  });

  it("treats an un-tuned budget as empty, never as a breach", () => {
    // An unconfigured threshold yields a non-finite ratio upstream. Screaming red at
    // config that was never set would train the desk to ignore the board.
    const buckets = bucketsFromIntents([
      intent("unset", "green", Number.POSITIVE_INFINITY),
      intent("nan", "green", Number.NaN),
    ]);
    for (const b of buckets) {
      expect(b.fill).toBe(0);
      expect(b.needsHedge).toBe(false);
      expect(Number.isFinite(b.utilization)).toBe(true);
    }
  });

  it("ranks an unknown band after every known one, never first", () => {
    expect(bandRank("breach")).toBeLessThan(bandRank("green"));
    expect(bandRank("something-new")).toBeGreaterThan(bandRank("green"));
  });
});

describe("hedgeBuckets — where the risk went", () => {
  it("sums the three disjoint legs of every fired hedge", () => {
    const flow = flowTotals([fire(2500, 1500, 0), fire(0, 500, 250)]);
    expect(flow).toMatchObject({
      crossed: 2500,
      hedged: 2000,
      warehoused: 250,
      fires: 2,
    });
  });

  it("EXCLUDES advisory fires — they never traded", () => {
    // Counting a dry run as shed risk would overstate what the desk actually did.
    const flow = flowTotals([
      fire(1000, 1000, 0),
      fire(9999, 9999, 9999, { advisory: true }),
    ]);
    expect(flow).toMatchObject({ crossed: 1000, hedged: 1000, fires: 1 });
  });

  it("collapses the bars rather than dividing by zero when nothing has fired", () => {
    expect(flowShares(flowTotals([]))).toEqual({
      crossed: 0,
      hedged: 0,
      warehoused: 0,
    });
  });

  it("splits shares proportionally", () => {
    const shares = flowShares(flowTotals([fire(50, 30, 20)]));
    expect(shares.crossed).toBeCloseTo(0.5, 10);
    expect(shares.hedged).toBeCloseTo(0.3, 10);
    expect(shares.warehoused).toBeCloseTo(0.2, 10);
  });
});

describe("hedgeBuckets — trade→hedge lineage", () => {
  const records = [
    fire(0, 1000, 0, { parent: 42n }),
    fire(0, 500, 0, { parent: 42n }),
    fire(0, 250, 0, { parent: 77n }),
    fire(2500, 0, 0), // book-level breach — no parent fill
  ];

  it("links a booked position to every hedge that offsets it", () => {
    expect(hedgesForPosition(records, 42n)).toHaveLength(2);
    expect(hedgesForPosition(records, 77n)).toHaveLength(1);
    expect(hedgesForPosition(records, 999n)).toHaveLength(0);
  });

  it("keeps parentless book-level fires in their own lane, not forced into a lineage", () => {
    // A book-level breach is keyed by book, not by a fill. Attaching it to some trade
    // would invent a link the data never claimed.
    const orphans = bookLevelFires(records);
    expect(orphans).toHaveLength(1);
    expect(orphans[0]?.internalCrossed).toBe(2500);
  });
});
