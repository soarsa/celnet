/**
 * Help-content registry: lookup, search ranking, referential integrity (every
 * feature/strategy id and every entry's tour resolves), and the in-editor
 * outbound-preview oracle (Flat ±25 price-bps ⇒ 99.30 / 99.80).
 */
import { describe, expect, test } from "vitest";

import {
  FEATURE_HELP_ID,
  getHelp,
  HELP_INDEX,
  helpByCategory,
  searchHelp,
  STRATEGY_HELP_ID,
} from "../src/lib/help";
import { getTour } from "../src/lib/tours";
import {
  defaultTieringConfig,
  outboundTwoWayPreview,
  TIERING_PREVIEW_RAW,
  defaultTieringStrategy,
} from "../src/lib/tiering";
import type { TieringConfig } from "../src/data/contract";

describe("help registry lookup", () => {
  test("resolves an entry by id with a complete worked example", () => {
    const e = getHelp("concept.bid-offer-tiering");
    expect(e).toBeDefined();
    expect(e?.title).toBe("Bid / offer tiering");
    expect(e?.example.rows.length).toBeGreaterThan(0);
    expect(e?.howToConfigure.length).toBeGreaterThan(0);
    expect(e?.risks).not.toHaveLength(0);
  });

  test("returns undefined for an unknown id", () => {
    expect(getHelp("nope.missing")).toBeUndefined();
  });

  test("every pricing-feature id resolves to an entry", () => {
    for (const id of Object.values(FEATURE_HELP_ID)) {
      expect(getHelp(id), `missing help entry ${id}`).toBeDefined();
    }
  });

  test("every tiering-strategy id resolves to an entry", () => {
    for (const id of Object.values(STRATEGY_HELP_ID)) {
      expect(getHelp(id), `missing help entry ${id}`).toBeDefined();
    }
  });

  test("every entry that references a tour points at a real tour", () => {
    for (const e of HELP_INDEX) {
      if (e.tourId) expect(getTour(e.tourId), `bad tour ${e.tourId} on ${e.id}`).toBeDefined();
    }
  });

  test("categories partition the whole index", () => {
    const total =
      helpByCategory("feature").length +
      helpByCategory("strategy").length +
      helpByCategory("concept").length;
    expect(total).toBe(HELP_INDEX.length);
  });
});

describe("help search", () => {
  test("“bid offer tiering” surfaces the bid/offer-tiering concept first", () => {
    const results = searchHelp("bid offer tiering");
    expect(results.length).toBeGreaterThan(0);
    expect(results[0]?.id).toBe("concept.bid-offer-tiering");
  });

  test("a strategy name finds its strategy entry", () => {
    const results = searchHelp("inventory skew");
    expect(results.map((r) => r.id)).toContain("strategy.inventory-skew");
  });

  test("an empty query returns the whole index in order", () => {
    expect(searchHelp("   ")).toHaveLength(HELP_INDEX.length);
  });

  test("a non-matching query returns nothing", () => {
    expect(searchHelp("zzzznotarealterm")).toHaveLength(0);
  });

  test("search is case-insensitive", () => {
    expect(searchHelp("TIERING").length).toBeGreaterThan(0);
  });
});

describe("outbound two-way preview (the worked-example oracle)", () => {
  test("default Flat ±25 price bps turns 99.50/99.60 into 99.30/99.80", () => {
    const out = outboundTwoWayPreview(defaultTieringConfig());
    expect(out.bid).toBeCloseTo(99.3, 6);
    expect(out.offer).toBeCloseTo(99.8, 6);
  });

  test("a null (disabled) config passes the raw composite through unchanged", () => {
    expect(outboundTwoWayPreview(null)).toEqual(TIERING_PREVIEW_RAW);
  });

  test("an inventory skew leans the whole two-way down for a long book", () => {
    const config: TieringConfig = {
      unit: "PRICE_POINTS",
      strategies: [
        { ...defaultTieringStrategy("FLAT_MARKUP"), halfSpread: 0.25 },
        {
          kind: "INVENTORY_SKEW",
          halfSpread: 0,
          kappa: 0.1,
          sMax: 0.5,
          smoothingWeight: 0,
          expectedSpread: 0,
          maxDivergence: 0,
          coreSpread: 0,
          maxOutputSpread: 0,
          spreadScaleFactor: 0,
        },
      ],
      guardrails: { hMin: 0, hMax: 1, sMax: 0.5, spreadFloor: 0.01 },
      stalePolicy: "SUPPRESS",
    };
    const out = outboundTwoWayPreview(config);
    // mid 99.55, h 0.25, skew 0.10 ⇒ bid 99.20 / offer 99.70 (spread kept, shifted down).
    expect(out.bid).toBeCloseTo(99.2, 6);
    expect(out.offer).toBeCloseTo(99.7, 6);
  });
});
