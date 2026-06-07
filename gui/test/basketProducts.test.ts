/**
 * Correlated multi-asset basket / best-of / worst-of for the GUI end of the ONE
 * `celnet.wire` contract (PC-BASKET). The Rust slice added a new `basket` product
 * arm at proto field 25 (after `american`=24) — APPENDED additively: no
 * `schema_version`, no renumber, the existing arms byte-identical (CLAUDE.md
 * rule 9). These tests exercise the REAL `src/data/seed.ts`, `src/data/enums.ts`
 * and `src/data/wsCodec.ts` through their public surface with NO server and NO
 * mocks.
 *
 * No offline pricing is claimed for the basket: it is a Monte-Carlo multi-asset
 * product priced by the live server (like TARF/accumulator), so the GUI's
 * contribution is the typed instrument + the EXACT wire encoding the server's
 * `basket_from_json` decodes. The proto contract's pinned field names / enum tags
 * are the independent oracle here.
 */
import { describe, expect, it } from "vitest";

import { instrumentToWire } from "../src/data/wsCodec";
import * as e from "../src/data/enums";
import { basketInstrument, type BasketTerms } from "../src/data/seed";
import type { CcyPair } from "../src/data/contract";

const SETTLE: CcyPair = { base: "EUR", quote: "USD" };

function terms(over: Partial<BasketTerms> = {}): BasketTerms {
  return {
    legs: [
      { pair: { base: "EUR", quote: "USD" }, weight: 0.5, spot: 1.1, vol: 0.11, rFor: 0.015 },
      { pair: { base: "GBP", quote: "USD" }, weight: 0.5, spot: 1.27, vol: 0.13, rFor: 0.02 },
    ],
    correlations: [1.0, 0.4, 0.4, 1.0],
    optionType: "CALL",
    strike: 1.18,
    kind: "WORST_OF",
    mcPaths: 8192,
    mcReplications: 16,
    mcSteps: 1,
    mcSeed: 0xc0ffeen,
    ...over,
  };
}

describe("basket-kind wire codec (proto enum numbers)", () => {
  it("maps BASKET↔0, BEST_OF↔1, WORST_OF↔2 reversibly", () => {
    expect(e.basketKind.toWire("BASKET")).toBe(0);
    expect(e.basketKind.toWire("BEST_OF")).toBe(1);
    expect(e.basketKind.toWire("WORST_OF")).toBe(2);
    expect(e.basketKind.fromWire(2)).toBe("WORST_OF");
    expect(e.basketKind.fromWire(99)).toBe("BASKET");
  });
});

describe("basket instrument → wire encoding (proto field-25 product contract)", () => {
  it("encodes the EXACT `basket` shape the server's basket_from_json decodes", () => {
    const instr = basketInstrument(SETTLE, 1, 1, terms());
    const wire = instrumentToWire(instr) as Record<string, unknown>;
    const basket = wire["basket"] as Record<string, unknown>;
    expect(basket).toBeDefined();

    const legs = basket["legs"] as Array<Record<string, unknown>>;
    expect(legs).toHaveLength(2);
    expect(legs[0]).toEqual({
      pair: { base: "EUR", quote: "USD" },
      weight: 0.5,
      spot: 1.1,
      vol: 0.11,
      r_for: 0.015,
    });
    expect(legs[1].r_for).toBe(0.02);

    expect(basket["correlations"]).toEqual([1.0, 0.4, 0.4, 1.0]);
    expect(basket["option_type"]).toBe(e.optionType.toWire("CALL"));
    expect(basket["kind"]).toBe(e.basketKind.toWire("WORST_OF"));
    expect(basket["strike"]).toBe(1.18);
    expect(basket["mc_paths"]).toBe(8192);
    expect(basket["mc_replications"]).toBe(16);
    expect(basket["mc_steps"]).toBe(1);
    expect(basket["mc_seed"]).toBe(0xc0ffeen);

    // The append is additive: the top-level settlement pair is preserved and no
    // other product key is present.
    expect(wire["pair"]).toEqual({ base: "EUR", quote: "USD" });
    expect(wire["american"]).toBeUndefined();
    expect(wire["vanilla"]).toBeUndefined();
  });

  it("round-trips the best-of and basket kinds", () => {
    for (const kind of ["BASKET", "BEST_OF", "WORST_OF"] as const) {
      const wire = instrumentToWire(basketInstrument(SETTLE, 1, 1, terms({ kind }))) as Record<
        string,
        unknown
      >;
      const basket = wire["basket"] as Record<string, unknown>;
      expect(basket["kind"]).toBe(e.basketKind.toWire(kind));
    }
  });
});
