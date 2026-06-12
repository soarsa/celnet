// Capability-matrix parity tests for `excel/src/taskpane/capability.ts`.
//
// These tests assert the invariants the task pane relies on to disable FX-only
// product arms on cross-asset underliers: the 24-arm FX/METAL full set, the
// 3-arm equity/commodity/crypto restricted set, and the `classForUnderlying`
// mapping from the `Underlying["kind"]` discriminant.  The assertions are the
// source of truth the pane reads — if a new arm or class lands on the wire
// contract, these tests fail immediately, forcing the matrix to be updated
// before the file compiles again.

import { describe, expect, it } from "vitest";

import {
  ALL_PRODUCT_KINDS,
  CROSS_ASSET_PRICEABLE,
  PRICEABLE,
  classForUnderlying,
  isPriceable,
  priceability,
} from "../src/taskpane/capability";
import type { AssetClass, ProductKind } from "../src/taskpane/capability";

// ---------------------------------------------------------------------------
// catalogue completeness
// ---------------------------------------------------------------------------

describe("ALL_PRODUCT_KINDS", () => {
  it("contains exactly 24 entries (all arms of the wire `product` oneof)", () => {
    expect(ALL_PRODUCT_KINDS).toHaveLength(24);
  });

  it("contains no duplicates", () => {
    expect(new Set(ALL_PRODUCT_KINDS).size).toBe(ALL_PRODUCT_KINDS.length);
  });

  it("covers every expected proto arm name", () => {
    const expected: readonly ProductKind[] = [
      "vanilla",
      "strategy",
      "singleBarrier",
      "doubleBarrier",
      "digital",
      "touch",
      "varianceSwap",
      "volatilitySwap",
      "asianOption",
      "forwardStart",
      "cliquet",
      "quanto",
      "tarf",
      "pivot",
      "accumulator",
      "lookback",
      "windowBarrier",
      "american",
      "basket",
      "fxForward",
      "fxSwap",
      "ndf",
      "perpetualOption",
      "listedFutureOption",
    ];
    expect([...ALL_PRODUCT_KINDS].sort()).toEqual([...expected].sort());
  });
});

describe("CROSS_ASSET_PRICEABLE", () => {
  it("contains exactly the 3 cross-asset arms", () => {
    expect(CROSS_ASSET_PRICEABLE).toHaveLength(3);
    expect([...CROSS_ASSET_PRICEABLE].sort()).toEqual(
      ["listedFutureOption", "perpetualOption", "vanilla"].sort(),
    );
  });
});

// ---------------------------------------------------------------------------
// PRICEABLE matrix — the core invariant the pane enforces
// ---------------------------------------------------------------------------

describe("PRICEABLE matrix", () => {
  it("FX prices all 24 arms", () => {
    expect(PRICEABLE["FX"].size).toBe(24);
    for (const kind of ALL_PRODUCT_KINDS) {
      expect(PRICEABLE["FX"].has(kind)).toBe(true);
    }
  });

  it("METAL prices all 24 arms (routes to the full FX engine)", () => {
    expect(PRICEABLE["METAL"].size).toBe(24);
    for (const kind of ALL_PRODUCT_KINDS) {
      expect(PRICEABLE["METAL"].has(kind)).toBe(true);
    }
  });

  it("EQUITY prices exactly vanilla + perpetualOption + listedFutureOption", () => {
    expect(PRICEABLE["EQUITY"].size).toBe(3);
    expect(PRICEABLE["EQUITY"].has("vanilla")).toBe(true);
    expect(PRICEABLE["EQUITY"].has("perpetualOption")).toBe(true);
    expect(PRICEABLE["EQUITY"].has("listedFutureOption")).toBe(true);
    // FX-only families are NOT in the set.
    for (const kind of ALL_PRODUCT_KINDS) {
      if (kind !== "vanilla" && kind !== "perpetualOption" && kind !== "listedFutureOption") {
        expect(PRICEABLE["EQUITY"].has(kind)).toBe(false);
      }
    }
  });

  it("COMMODITY prices exactly vanilla + perpetualOption + listedFutureOption", () => {
    expect(PRICEABLE["COMMODITY"].size).toBe(3);
    expect(PRICEABLE["COMMODITY"].has("vanilla")).toBe(true);
    expect(PRICEABLE["COMMODITY"].has("perpetualOption")).toBe(true);
    expect(PRICEABLE["COMMODITY"].has("listedFutureOption")).toBe(true);
    for (const kind of ALL_PRODUCT_KINDS) {
      if (kind !== "vanilla" && kind !== "perpetualOption" && kind !== "listedFutureOption") {
        expect(PRICEABLE["COMMODITY"].has(kind)).toBe(false);
      }
    }
  });

  it("CRYPTO prices exactly vanilla + perpetualOption + listedFutureOption", () => {
    expect(PRICEABLE["CRYPTO"].size).toBe(3);
    expect(PRICEABLE["CRYPTO"].has("vanilla")).toBe(true);
    expect(PRICEABLE["CRYPTO"].has("perpetualOption")).toBe(true);
    expect(PRICEABLE["CRYPTO"].has("listedFutureOption")).toBe(true);
    for (const kind of ALL_PRODUCT_KINDS) {
      if (kind !== "vanilla" && kind !== "perpetualOption" && kind !== "listedFutureOption") {
        expect(PRICEABLE["CRYPTO"].has(kind)).toBe(false);
      }
    }
  });

  it("EQUITY / COMMODITY / CRYPTO are all identical priceable sets", () => {
    const eq = [...PRICEABLE["EQUITY"]].sort();
    const co = [...PRICEABLE["COMMODITY"]].sort();
    const cr = [...PRICEABLE["CRYPTO"]].sort();
    expect(eq).toEqual(co);
    expect(eq).toEqual(cr);
  });

  it("cross-asset sets are a strict subset of FX (FX is a superset)", () => {
    for (const cls of ["EQUITY", "COMMODITY", "CRYPTO"] as const) {
      for (const kind of PRICEABLE[cls]) {
        expect(PRICEABLE["FX"].has(kind)).toBe(true);
      }
    }
  });
});

// ---------------------------------------------------------------------------
// classForUnderlying — Underlying["kind"] → AssetClass
// ---------------------------------------------------------------------------

describe("classForUnderlying", () => {
  it("maps fx → FX", () => {
    expect(classForUnderlying("fx")).toBe("FX");
  });

  it("maps metal → METAL", () => {
    expect(classForUnderlying("metal")).toBe("METAL");
  });

  it("maps equity → EQUITY", () => {
    expect(classForUnderlying("equity")).toBe("EQUITY");
  });

  it("maps commodity → COMMODITY", () => {
    expect(classForUnderlying("commodity")).toBe("COMMODITY");
  });

  it("maps digitalAsset → CRYPTO", () => {
    expect(classForUnderlying("digitalAsset")).toBe("CRYPTO");
  });

  it("covers all five underlying kinds (exhaustive switch)", () => {
    // TypeScript's exhaustive switch guarantees this at compile time; the test
    // proves the runtime values match.
    const pairs: Array<[Parameters<typeof classForUnderlying>[0], AssetClass]> = [
      ["fx", "FX"],
      ["metal", "METAL"],
      ["equity", "EQUITY"],
      ["commodity", "COMMODITY"],
      ["digitalAsset", "CRYPTO"],
    ];
    for (const [kind, expected] of pairs) {
      expect(classForUnderlying(kind)).toBe(expected);
    }
  });
});

// ---------------------------------------------------------------------------
// isPriceable / priceability helpers
// ---------------------------------------------------------------------------

describe("isPriceable", () => {
  it("returns true for vanilla on every asset class", () => {
    const classes: AssetClass[] = ["FX", "METAL", "EQUITY", "COMMODITY", "CRYPTO"];
    for (const cls of classes) {
      expect(isPriceable("vanilla", cls)).toBe(true);
    }
  });

  it("returns false for a barrier on EQUITY / COMMODITY / CRYPTO", () => {
    expect(isPriceable("singleBarrier", "EQUITY")).toBe(false);
    expect(isPriceable("singleBarrier", "COMMODITY")).toBe(false);
    expect(isPriceable("singleBarrier", "CRYPTO")).toBe(false);
  });

  it("returns true for a barrier on FX and METAL", () => {
    expect(isPriceable("singleBarrier", "FX")).toBe(true);
    expect(isPriceable("singleBarrier", "METAL")).toBe(true);
  });

  it("returns true for perpetualOption on every asset class", () => {
    const classes: AssetClass[] = ["FX", "METAL", "EQUITY", "COMMODITY", "CRYPTO"];
    for (const cls of classes) {
      expect(isPriceable("perpetualOption", cls)).toBe(true);
    }
  });

  it("returns true for listedFutureOption on every asset class", () => {
    const classes: AssetClass[] = ["FX", "METAL", "EQUITY", "COMMODITY", "CRYPTO"];
    for (const cls of classes) {
      expect(isPriceable("listedFutureOption", cls)).toBe(true);
    }
  });
});

describe("priceability", () => {
  it("returns 'priceable' for vanilla on every class", () => {
    const classes: AssetClass[] = ["FX", "METAL", "EQUITY", "COMMODITY", "CRYPTO"];
    for (const cls of classes) {
      expect(priceability("vanilla", cls)).toBe("priceable");
    }
  });

  it("returns a Dimmed object (dimmed: true) for a cross-asset-restricted arm on EQUITY", () => {
    const result = priceability("tarf", "EQUITY");
    expect(result).not.toBe("priceable");
    if (result !== "priceable") {
      expect(result.dimmed).toBe(true);
      // The reason message mirrors the instrumentSpec.ts guard's honest language.
      expect(result.reason).toContain("FX/metal-only");
      expect(result.reason).toContain("vanilla");
      expect(result.reason).toContain("tarf");
    }
  });

  it("includes the class name in the dim reason for COMMODITY and CRYPTO", () => {
    const co = priceability("lookback", "COMMODITY");
    const cr = priceability("lookback", "CRYPTO");
    if (co !== "priceable") {
      expect(co.reason.toLowerCase()).toContain("commodity");
    }
    if (cr !== "priceable") {
      expect(cr.reason.toLowerCase()).toContain("crypto");
    }
  });

  it("returns 'priceable' (not Dimmed) for barrier on FX and METAL", () => {
    expect(priceability("singleBarrier", "FX")).toBe("priceable");
    expect(priceability("singleBarrier", "METAL")).toBe("priceable");
  });

  it("returns Dimmed for every FX-only arm on EQUITY with the correct shape", () => {
    const fxOnlyArms: ProductKind[] = ALL_PRODUCT_KINDS.filter(
      (k) => !PRICEABLE["EQUITY"].has(k),
    );
    expect(fxOnlyArms.length).toBeGreaterThan(0);
    for (const kind of fxOnlyArms) {
      const result = priceability(kind, "EQUITY");
      expect(result).not.toBe("priceable");
      if (result !== "priceable") {
        expect(result.dimmed).toBe(true);
        expect(typeof result.reason).toBe("string");
        expect(result.reason.length).toBeGreaterThan(0);
      }
    }
  });
});
