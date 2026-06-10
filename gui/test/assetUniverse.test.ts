/**
 * assetUniverse tests — the cross-class underlier-universe model behind the scope
 * drill's asset-class rail (book → asset class → underlier):
 *
 *   • classification + the pair PROJECTION (metal legs become ISO-4217 X-codes —
 *     re-derived here against the published code table, NOT read back from the
 *     production projection, so a symmetric mapping bug cannot pass);
 *   • the seeded universes' load-bearing contents (the four metals vs majors +
 *     the metal crosses; equities with venue + ccy; crypto linear vs inverse);
 *   • the fuzzy search grammar (same as the FX pair universe's);
 *   • the ticket pre-target ROUND-TRIP LAW: `crossAssetInputsFor` is the exact
 *     inverse of the cross-asset spec's `crossAssetUnderlying` wire seam for
 *     every seeded underlier — proving a universe selection re-uses the ONE
 *     wire-building path rather than duplicating it.
 */
import { describe, expect, it } from "vitest";

import { ASSET_UNDERLIERS } from "../src/data/assetUniverse";
import {
  ASSET_CLASSES,
  ASSET_CLASS_LABEL,
  buildAssetUniverse,
  searchUnderliers,
  toUnderlierRow,
  underlierAssetClass,
} from "../src/lib/assetUniverse";
import {
  crossAssetInputsFor,
  crossAssetSettlement,
  crossAssetUnderlying,
} from "../src/products/crossAsset";
import type { Underlying } from "../src/data/contract";

const universe = buildAssetUniverse(ASSET_UNDERLIERS);

describe("asset-class vocabulary", () => {
  it("the rail is FX-first with all five classes", () => {
    expect(ASSET_CLASSES).toEqual(["FX", "METAL", "EQUITY", "COMMODITY", "CRYPTO"]);
    for (const c of ASSET_CLASSES) expect(ASSET_CLASS_LABEL[c].length).toBeGreaterThan(0);
  });

  it("classifies every Underlying arm", () => {
    const fx: Underlying = { kind: "fx", fx: { base: "EUR", quote: "USD" }, settlementCcy: "USD" };
    expect(underlierAssetClass(fx)).toBe("FX");
    expect(
      underlierAssetClass({
        kind: "metal",
        metal: { metal: "GOLD", quote: "USD" },
        settlementCcy: "USD",
      }),
    ).toBe("METAL");
    expect(
      underlierAssetClass({
        kind: "equity",
        equity: { symbol: { ticker: "AAPL", venue: "XNAS" }, currency: "USD" },
        settlementCcy: "USD",
      }),
    ).toBe("EQUITY");
    expect(
      underlierAssetClass({
        kind: "commodity",
        commodity: { symbol: { ticker: "BRENT", venue: "IFEU" }, currency: "USD" },
        settlementCcy: "USD",
      }),
    ).toBe("COMMODITY");
    expect(
      underlierAssetClass({
        kind: "digitalAsset",
        digitalAsset: { base: "BTC", quote: "USDT" },
        settlementCcy: "USDT",
      }),
    ).toBe("CRYPTO");
  });
});

describe("buildAssetUniverse — per-class grouping, projection ids, no FX rows", () => {
  it("groups every seeded row under its non-FX class with unique projected ids", () => {
    const classCounts = [...universe.byClass.entries()].map(([c, rows]) => [c, rows.length]);
    for (const [, n] of classCounts) expect(n).toBeGreaterThan(0);
    expect(universe.byClass.has("FX")).toBe(false);
    const ids = universe.all.map((r) => r.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const r of universe.all) expect(universe.byId.get(r.id)).toBe(r);
  });

  it("metal rows project the metal leg to its ISO-4217 X-code (independent table)", () => {
    // The published X-code table, re-stated here (NOT read from the production
    // projection) so a symmetric mapping bug cannot pass.
    const X_CODE = { GOLD: "XAU", SILVER: "XAG", PLATINUM: "XPT", PALLADIUM: "XPD" } as const;
    const metals = universe.byClass.get("METAL")!;
    expect(metals.length).toBeGreaterThan(0);
    for (const r of metals) {
      if (r.underlying.kind !== "metal") throw new Error("non-metal row in METAL class");
      const code = X_CODE[r.underlying.metal.metal];
      expect(r.id.startsWith(code)).toBe(true);
      expect(r.label).toBe(`${code}/${r.underlying.metal.quote}`);
    }
  });

  it("seeds all four metals vs USD plus the metal-vs-fiat crosses", () => {
    const ids = new Set(universe.byClass.get("METAL")!.map((r) => r.id));
    for (const id of ["XAUUSD", "XAGUSD", "XPTUSD", "XPDUSD"]) expect(ids.has(id)).toBe(true);
    // The metal-vs-FIAT crosses the conventions registry carries (XAUEUR/XPTEUR…).
    expect(ids.has("XAUEUR")).toBe(true);
    expect(ids.has("XPTEUR")).toBe(true);
    // Metal-vs-METAL ratios are NOT priceable (registry is metal-vs-fiat only,
    // MetalPair.quote is a fiat Ccy) and must never be seeded (honesty rule).
    expect(ids.has("XAUXAG")).toBe(false);
    expect(ids.has("XPTXPD")).toBe(false);
  });

  it("equity/commodity rows are ticker-labelled and carry venue · ccy detail", () => {
    for (const cls of ["EQUITY", "COMMODITY"] as const) {
      for (const r of universe.byClass.get(cls)!) {
        const u = r.underlying;
        const ref =
          u.kind === "equity" ? u.equity : u.kind === "commodity" ? u.commodity : null;
        if (!ref) throw new Error(`non-${cls} row in ${cls} class`);
        expect(r.label).toBe(ref.symbol.ticker);
        expect(r.detail).toBe(`${ref.symbol.venue} · ${ref.currency}`);
      }
    }
  });

  it("crypto rows note linear (stable-quoted) vs inverse (coin-margined, USD-quoted)", () => {
    const crypto = universe.byClass.get("CRYPTO")!;
    const byId = new Map(crypto.map((r) => [r.id, r]));
    for (const base of ["BTC", "ETH"]) {
      const usd = byId.get(`${base}USD`)!;
      const usdt = byId.get(`${base}USDT`)!;
      expect(usd.settlementStyle).toBe("INVERSE_COIN");
      expect(usd.detail).toContain("inverse");
      expect(usdt.settlementStyle).toBe("LINEAR");
      expect(usdt.detail).toContain("linear");
    }
  });

  it("a row's haystack covers id, pair form, legs, detail and class label", () => {
    const row = toUnderlierRow(ASSET_UNDERLIERS[0]!); // XAU/USD
    expect(row.haystack).toContain("xauusd");
    expect(row.haystack).toContain("xau/usd");
    expect(row.haystack).toContain("usd");
    expect(row.haystack).toContain("metals");
  });
});

describe("searchUnderliers — the same fuzzy grammar as the FX universe", () => {
  const metals = universe.byClass.get("METAL")!;

  it("an empty query returns every row in seed order with no highlight", () => {
    const hits = searchUnderliers(metals, "  ");
    expect(hits.map((h) => h.row.id)).toEqual(metals.map((r) => r.id));
    expect(hits.every((h) => h.score === 0 && h.indices.length === 0)).toBe(true);
  });

  it("matches a leg / ticker query and ranks the contiguous hit on top", () => {
    const hits = searchUnderliers(metals, "xpd");
    expect(hits.length).toBeGreaterThan(0);
    expect(hits[0]!.row.id).toBe("XPDUSD");
    const equities = universe.byClass.get("EQUITY")!;
    expect(searchUnderliers(equities, "nvda")[0]!.row.label).toBe("NVDA");
  });

  it("a detail-only hit is kept with NO fabricated label highlight", () => {
    const crypto = universe.byClass.get("CRYPTO")!;
    const hits = searchUnderliers(crypto, "inverse");
    expect(hits.length).toBe(2); // the two coin-margined rows
    expect(hits.every((h) => h.row.settlementStyle === "INVERSE_COIN")).toBe(true);
    expect(hits.every((h) => h.indices.length === 0)).toBe(true);
  });

  it("drops rows the query cannot subsequence", () => {
    expect(searchUnderliers(metals, "zzzqq")).toHaveLength(0);
  });
});

describe("ticket pre-target round-trip law (no duplicated wire-building)", () => {
  it("crossAssetInputsFor inverts crossAssetUnderlying for EVERY seeded underlier", () => {
    for (const seed of ASSET_UNDERLIERS) {
      const inputs = crossAssetInputsFor(seed.underlying, seed.settlementStyle);
      expect(inputs).not.toBeNull();
      // The spec's own wire seam rebuilds the EXACT contract identity…
      expect(crossAssetUnderlying(inputs!)).toEqual(seed.underlying);
      // …and the settlement mechanics: crypto keeps the seeded style, every other
      // class is forced LINEAR (a coin-margined flag never leaks cross-class).
      const expected =
        seed.underlying.kind === "digitalAsset" ? seed.settlementStyle : "LINEAR";
      expect(crossAssetSettlement(inputs!)).toBe(expected);
    }
  });

  it("an FX underlying never re-points the ticket (null pre-target)", () => {
    const fx: Underlying = { kind: "fx", fx: { base: "EUR", quote: "USD" }, settlementCcy: "USD" };
    expect(crossAssetInputsFor(fx, "LINEAR")).toBeNull();
  });
});
