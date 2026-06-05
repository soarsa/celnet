/**
 * universe.ts tests — the registry-ready pair-universe model over the seeded
 * pairs: desk-taxonomy classification (major/cross/EM), grouping, the searchable
 * haystack, and the fuzzy search/regroup. Pure + deterministic; exercises the
 * REAL `src/lib/universe.ts` (and the `src/lib/fuzzy.ts` it builds on).
 */
import { describe, expect, it } from "vitest";

import {
  bucketOf,
  buildUniverse,
  groupHits,
  pairId,
  pairLabel,
  samePair,
  searchUniverse,
  toUniversePair,
} from "../src/lib/universe";
import type { PairContext } from "../src/data/seed";

const market = { spot: 1, vol: 0.08, rDom: 0, rFor: 0 };
function ctx(base: string, quote: string): PairContext {
  return { pair: { base, quote }, market, pipDecimals: 4 };
}

describe("classification — desk taxonomy from the two currency codes", () => {
  it("buckets a G10 dollar pair as a major", () => {
    expect(bucketOf({ base: "EUR", quote: "USD" })).toBe("major");
    expect(bucketOf({ base: "USD", quote: "JPY" })).toBe("major");
  });

  it("buckets a G10×G10 non-USD pair as a cross", () => {
    expect(bucketOf({ base: "EUR", quote: "GBP" })).toBe("cross");
    expect(bucketOf({ base: "EUR", quote: "JPY" })).toBe("cross");
  });

  it("buckets anything touching a non-G10 currency as EM", () => {
    expect(bucketOf({ base: "USD", quote: "TRY" })).toBe("em");
    expect(bucketOf({ base: "EUR", quote: "ZAR" })).toBe("em");
  });
});

describe("pair identity helpers", () => {
  it("labels + keys a pair canonically", () => {
    expect(pairLabel({ base: "EUR", quote: "USD" })).toBe("EUR/USD");
    expect(pairId({ base: "EUR", quote: "USD" })).toBe("EURUSD");
  });

  it("samePair compares both legs", () => {
    expect(samePair({ base: "EUR", quote: "USD" }, { base: "EUR", quote: "USD" })).toBe(true);
    expect(samePair({ base: "EUR", quote: "USD" }, { base: "USD", quote: "EUR" })).toBe(false);
  });

  it("toUniversePair builds a lower-cased haystack covering id, form and bare legs", () => {
    const u = toUniversePair(ctx("EUR", "USD"));
    expect(u.haystack).toContain("eurusd");
    expect(u.haystack).toContain("eur/usd");
    expect(u.haystack).toContain("eur");
    expect(u.haystack).toContain("usd");
    expect(u.bucket).toBe("major");
  });
});

describe("buildUniverse — grouped, indexed, source-ordered", () => {
  const pairs = [ctx("EUR", "USD"), ctx("EUR", "GBP"), ctx("USD", "TRY")];
  const u = buildUniverse(pairs);

  it("keeps all pairs in source order with an O(1) byId index", () => {
    expect(u.all.map((p) => p.id)).toEqual(["EURUSD", "EURGBP", "USDTRY"]);
    expect(u.byId.get("EURGBP")?.bucket).toBe("cross");
  });

  it("groups into the canonical major/cross/EM order, dropping empty buckets", () => {
    expect(u.groups.map((g) => g.bucket)).toEqual(["major", "cross", "em"]);
    expect(u.groups.find((g) => g.bucket === "major")?.pairs.map((p) => p.id)).toEqual([
      "EURUSD",
    ]);
  });
});

describe("searchUniverse — fuzzy match + ranking", () => {
  const u = buildUniverse([ctx("EUR", "USD"), ctx("GBP", "USD"), ctx("USD", "JPY")]);

  it("an empty query returns every pair in source order with no highlight", () => {
    const hits = searchUniverse(u, "   ");
    expect(hits.map((h) => h.pair.id)).toEqual(["EURUSD", "GBPUSD", "USDJPY"]);
    expect(hits.every((h) => h.score === 0 && h.indices.length === 0)).toBe(true);
  });

  it("matches on the bare leg and ranks an exact-ish hit at the top", () => {
    const hits = searchUniverse(u, "eur");
    expect(hits[0]?.pair.id).toBe("EURUSD");
  });

  it("matches a contiguous id query and returns label highlight indices", () => {
    const hits = searchUniverse(u, "gbpusd");
    expect(hits[0]?.pair.id).toBe("GBPUSD");
    // "gbpusd" subsequences the visible label "GBP/USD" ⇒ non-empty highlight.
    expect(hits[0]?.indices.length).toBeGreaterThan(0);
  });

  it("drops a pair the query cannot subsequence", () => {
    const hits = searchUniverse(u, "zzz");
    expect(hits).toHaveLength(0);
  });

  it("groupHits regroups a flat hit list into canonical buckets", () => {
    const grouped = groupHits(searchUniverse(u, "usd"));
    // EUR/USD, GBP/USD, USD/JPY are all USD majors.
    expect(grouped.map((g) => g.bucket)).toEqual(["major"]);
    expect(grouped[0]?.hits.length).toBe(3);
  });
});
