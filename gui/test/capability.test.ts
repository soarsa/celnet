/**
 * Cross-asset capability-matrix contract test. The matrix (`capability.ts`) is the
 * single executable source for "which product arm prices on which asset class" that
 * the discovery layer + class-aware analytics + Excel port all read; this gate pins
 * it to the authoritative boundary (docs/EXCEL-INTEGRATION.md §3.3.1 = the server's
 * `price_cross_asset` + the Excel build-time guard): FX/METAL price all 24 arms;
 * equity/commodity/crypto price only vanilla + perpetual + listed-future-option.
 */
import { describe, expect, it } from "vitest";

import {
  ALL_PRODUCT_KINDS,
  CROSS_ASSET_PRICEABLE,
  PRICEABLE,
  classForUnderlying,
  isPriceable,
  priceability,
  type ProductKind,
} from "../src/products/capability";
import type { AssetClass } from "../src/products/types";
import type { Underlying } from "../src/data/contract";

const CROSS_ASSET: readonly AssetClass[] = ["EQUITY", "COMMODITY", "CRYPTO"];
const FX_LIKE: readonly AssetClass[] = ["FX", "METAL"];

describe("capability matrix — coverage", () => {
  it("enumerates the 24 product-oneof arms", () => {
    expect(ALL_PRODUCT_KINDS.length).toBe(24);
    expect(new Set(ALL_PRODUCT_KINDS).size).toBe(24); // no duplicates
  });

  it("FX and METAL price every arm (full FX engine)", () => {
    for (const cls of FX_LIKE) {
      for (const kind of ALL_PRODUCT_KINDS) {
        expect(isPriceable(kind, cls)).toBe(true);
        expect(priceability(kind, cls)).toBe("priceable");
      }
      expect(PRICEABLE[cls].size).toBe(24);
    }
  });

  it("equity/commodity/crypto price ONLY the carry leaves + agnostic arms", () => {
    for (const cls of CROSS_ASSET) {
      expect([...PRICEABLE[cls]].sort()).toEqual([...CROSS_ASSET_PRICEABLE].sort());
      expect(isPriceable("vanilla", cls)).toBe(true);
      expect(isPriceable("perpetualOption", cls)).toBe(true);
      expect(isPriceable("listedFutureOption", cls)).toBe(true);
    }
  });

  it("FX-only exotics are dimmed-with-reason on a cross-asset class", () => {
    const fxOnly: ProductKind[] = ALL_PRODUCT_KINDS.filter(
      (k) => !(CROSS_ASSET_PRICEABLE as readonly string[]).includes(k),
    );
    expect(fxOnly.length).toBe(21);
    for (const cls of CROSS_ASSET) {
      for (const kind of fxOnly) {
        expect(isPriceable(kind, cls)).toBe(false);
        const v = priceability(kind, cls);
        expect(v).not.toBe("priceable");
        if (v !== "priceable") {
          expect(v.dimmed).toBe(true);
          expect(v.reason).toMatch(/only vanilla, perpetual and listed-future-option/);
          expect(v.reason).toMatch(/FX\/metal-only/);
        }
      }
    }
  });
});

describe("capability matrix — classForUnderlying", () => {
  it("maps each Underlying arm to its trader-facing class", () => {
    const cases: Array<[Underlying["kind"], AssetClass]> = [
      ["fx", "FX"],
      ["metal", "METAL"],
      ["equity", "EQUITY"],
      ["commodity", "COMMODITY"],
      ["digitalAsset", "CRYPTO"],
    ];
    for (const [kind, cls] of cases) {
      expect(classForUnderlying(kind)).toBe(cls);
    }
  });
});
