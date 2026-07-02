/**
 * Cross-asset integration parity for the GUI end of the ONE `celnet.wire` contract:
 * the equity / commodity / digital-asset (crypto) / metal underlyings surfaced as a
 * single `crossAssetSpec` {@link ProductSpec} over the W1 generalized `Underlying`
 * oneof (`Instrument.underlying`, proto field 1) + `Instrument.settlement_style`
 * (proto field 29). Each books under the `product.vanilla` arm, carrying its
 * asset-class identity on `underlying` (the active arm by proto field NAME —
 * `equity`/`commodity`/`digital_asset`/`metal`, oneof `ref` field numbers
 * 4/5/6/3) and, for a coin-margined crypto, INVERSE_COIN on `settlement_style`.
 *
 * Exercises the REAL `src/data/seed.ts` builder, the `crossAssetSpec` `toInstrument`
 * builder, and the real `src/data/wsCodec.ts` / `src/data/enums.ts` through their
 * public surface with NO server and NO mocks. Pins the wire shape of each arm, the
 * FX-pair leg-string projection, LINEAR presence-omission, the INVERSE_COIN coin-
 * margin flag, and the new gallery group.
 */
import { describe, expect, it } from "vitest";

import {
  crossAssetSpec,
  crossAssetSettlement,
  crossAssetUnderlying,
  type CrossAssetInputs,
} from "../src/products/crossAsset";
import { registryByGroup, type ProductBuildCtx } from "../src/products";
import { crossAssetVanillaInstrument, underlyingPairProjection } from "../src/data/seed";
import { instrumentToWire } from "../src/data/wsCodec";
import * as e from "../src/data/enums";

const CTX: ProductBuildCtx = {
  pair: { base: "EUR", quote: "USD" },
  tenor: { unit: "MONTHS", count: 3 },
  tenorYears: 0.25,
  notionalMm: 10,
  pricingModel: "DEFAULT",
  atmForward: 1.085,
  spot: 1.08,
  pipDecimals: 4,
  today: { year: 2026, month: 6, day: 9 },
};

// ---------------------------------------------------------------------------
// underlying oneof arms — encoded by proto field NAME + settlement_ccy
// ---------------------------------------------------------------------------

describe("cross-asset Underlying oneof — instrumentToWire", () => {
  it("encodes an equity underlying under `equity` with the nested symbol + currency", () => {
    const inst = crossAssetVanillaInstrument(0.25, 10, {
      optionType: "CALL",
      strike: { kind: "strike", strike: 200 },
      underlying: {
        kind: "equity",
        equity: { symbol: { ticker: "AAPL", venue: "XNAS" }, currency: "USD" },
        settlementCcy: "USD",
      },
      settlementStyle: "LINEAR",
    });
    // FX pair projection keeps the FX-keyed surfaces total.
    expect(inst.pair).toEqual({ base: "AAPL", quote: "USD" });
    const w = instrumentToWire(inst);
    expect(w["underlying"]).toEqual({
      equity: { symbol: { ticker: "AAPL", venue: "XNAS" }, currency: "USD" },
      settlement_ccy: "USD",
    });
    expect("settlement_style" in w).toBe(false);
    expect(w["vanilla"]).toBeTruthy();
  });

  it("encodes a commodity underlying under `commodity`", () => {
    const inst = crossAssetVanillaInstrument(0.5, 5, {
      optionType: "PUT",
      strike: { kind: "strike", strike: 85 },
      underlying: {
        kind: "commodity",
        commodity: { symbol: { ticker: "BRENT", venue: "" }, currency: "USD" },
        settlementCcy: "USD",
      },
      settlementStyle: "LINEAR",
    });
    const w = instrumentToWire(inst);
    expect(w["underlying"]).toEqual({
      commodity: { symbol: { ticker: "BRENT", venue: "" }, currency: "USD" },
      settlement_ccy: "USD",
    });
  });

  it("encodes a LINEAR crypto underlying under `digital_asset` with settlement_style omitted", () => {
    const inst = crossAssetVanillaInstrument(1 / 12, 2, {
      optionType: "CALL",
      strike: { kind: "strike", strike: 70000 },
      underlying: {
        kind: "digitalAsset",
        digitalAsset: { base: "BTC", quote: "USDT" },
        settlementCcy: "USDT",
      },
      settlementStyle: "LINEAR",
    });
    const w = instrumentToWire(inst);
    expect(w["underlying"]).toEqual({
      digital_asset: { base: "BTC", quote: "USDT" },
      settlement_ccy: "USDT",
    });
    expect("settlement_style" in w).toBe(false);
  });

  it("carries settlement_style=INVERSE_COIN for a coin-margined crypto underlying", () => {
    const inst = crossAssetVanillaInstrument(0.25, 1, {
      optionType: "PUT",
      strike: { kind: "strike", strike: 65000 },
      underlying: {
        kind: "digitalAsset",
        digitalAsset: { base: "BTC", quote: "USD" },
        settlementCcy: "USD",
      },
      settlementStyle: "INVERSE_COIN",
    });
    expect(inst.settlementStyle).toBe("INVERSE_COIN");
    const w = instrumentToWire(inst);
    expect(w["settlement_style"]).toBe(e.settlementStyle.toWire("INVERSE_COIN"));
  });

  it("projects a metal leg onto its ISO X-code and encodes the numeric Metal tag", () => {
    const inst = crossAssetVanillaInstrument(1, 100, {
      optionType: "CALL",
      strike: { kind: "strike", strike: 2400 },
      underlying: { kind: "metal", metal: { metal: "GOLD", quote: "USD" }, settlementCcy: "USD" },
      settlementStyle: "LINEAR",
    });
    expect(inst.pair).toEqual({ base: "XAU", quote: "USD" });
    const w = instrumentToWire(inst);
    expect(w["underlying"]).toEqual({
      metal: { metal: e.metal.toWire("GOLD"), quote: "USD" },
      settlement_ccy: "USD",
    });
  });

  it("pins the canonical Metal / SettlementStyle enum numbers the server decodes by", () => {
    expect(e.metal.toWire("GOLD")).toBe(0);
    expect(e.metal.toWire("PALLADIUM")).toBe(3);
    expect(e.settlementStyle.toWire("LINEAR")).toBe(0);
    expect(e.settlementStyle.toWire("INVERSE_COIN")).toBe(1);
    expect(e.settlementStyle.fromWire(1)).toBe("INVERSE_COIN");
  });

  it("projects every Underlying arm onto a leg-string pair", () => {
    expect(underlyingPairProjection({ kind: "fx", fx: { base: "EUR", quote: "USD" }, settlementCcy: "USD" })).toEqual({
      base: "EUR",
      quote: "USD",
    });
    expect(
      underlyingPairProjection({ kind: "metal", metal: { metal: "SILVER", quote: "USD" }, settlementCcy: "USD" }),
    ).toEqual({ base: "XAG", quote: "USD" });
  });
});

// ---------------------------------------------------------------------------
// crossAssetSpec — the ProductSpec round-trip over the asset-class selector
// ---------------------------------------------------------------------------

describe("crossAssetSpec — toInstrument over the asset-class selector", () => {
  it("the default (equity) books a vanilla on an equity underlying", () => {
    const inst = crossAssetSpec.toInstrument(crossAssetSpec.defaults, CTX);
    expect(inst.product.kind).toBe("vanilla");
    expect(inst.underlying?.kind).toBe("equity");
    expect(inst.tenor).toEqual(CTX.tenor);
    // DEFAULT model is presence-omitted.
    expect(inst.pricingModel).toBeUndefined();
  });

  it("a CRYPTO + INVERSE_COIN selection books the coin-margined arm; other classes force LINEAR", () => {
    const cryptoInverse: CrossAssetInputs = {
      ...crossAssetSpec.defaults,
      assetKind: "CRYPTO",
      symbol: "BTC",
      currency: "USD",
      settlementStyle: "INVERSE_COIN",
    };
    expect(crossAssetSettlement(cryptoInverse)).toBe("INVERSE_COIN");
    const inst = crossAssetSpec.toInstrument(cryptoInverse, CTX);
    expect(inst.underlying?.kind).toBe("digitalAsset");
    expect(inst.settlementStyle).toBe("INVERSE_COIN");

    // INVERSE_COIN is forced to LINEAR for a non-crypto class (never leaks).
    const equityInverse: CrossAssetInputs = { ...cryptoInverse, assetKind: "EQUITY" };
    expect(crossAssetSettlement(equityInverse)).toBe("LINEAR");
    expect(crossAssetSpec.toInstrument(equityInverse, CTX).settlementStyle).toBeUndefined();
  });

  it("crossAssetUnderlying builds the arm matching the selected asset class", () => {
    expect(crossAssetUnderlying({ ...crossAssetSpec.defaults, assetKind: "COMMODITY", symbol: "brent", currency: "usd" }))
      .toEqual({
        kind: "commodity",
        commodity: { symbol: { ticker: "BRENT", venue: "XNAS" }, currency: "USD" },
        settlementCcy: "USD",
      });
    expect(crossAssetUnderlying({ ...crossAssetSpec.defaults, assetKind: "METAL", metal: "PLATINUM", currency: "USD" }))
      .toEqual({ kind: "metal", metal: { metal: "PLATINUM", quote: "USD" }, settlementCcy: "USD" });
  });

  it("round-trips deterministically through the real wire codec for every asset class", () => {
    const classes: CrossAssetInputs["assetKind"][] = ["EQUITY", "COMMODITY", "CRYPTO", "METAL"];
    for (const assetKind of classes) {
      const inst = crossAssetSpec.toInstrument({ ...crossAssetSpec.defaults, assetKind }, CTX);
      const wire = instrumentToWire(inst);
      expect(wire["underlying"]).toBeTruthy();
      expect(instrumentToWire(inst)).toEqual(wire);
    }
  });
});

// ---------------------------------------------------------------------------
// gallery grouping — the new cross-asset group
// ---------------------------------------------------------------------------

describe("cross-asset gallery group", () => {
  it("the cross-asset spec appears under its own group, ordered ahead of the fixed-income fold", () => {
    const grouped = registryByGroup();
    const groupOrder = grouped.map((g) => g.group);
    const xasset = grouped.find((g) => g.group === "Cross-asset (equity / commodity / crypto)");
    expect(xasset).toBeDefined();
    expect(xasset!.specs.map((s) => s.id)).toContain("CROSS_ASSET_VANILLA");
    // fe-fi-migration #3 appended the fixed-income (rates) family LAST in canonical
    // order, so cross-asset now sits immediately before it (not the terminal group).
    const xassetIdx = groupOrder.indexOf("Cross-asset (equity / commodity / crypto)");
    const fiIdx = groupOrder.indexOf("Fixed income (rates)");
    expect(fiIdx).toBeGreaterThan(xassetIdx);
    expect(groupOrder[groupOrder.length - 1]).toBe("Fixed income (rates)");
  });
});
