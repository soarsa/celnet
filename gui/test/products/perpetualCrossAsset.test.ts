/**
 * Cross-asset gate for the perpetual (no-expiry) American option — proving the
 * asset-class-AGNOSTIC arm (proto `perpetual_option`, field 30) BUILDS on an
 * equity / commodity / crypto underlier and carries the correct cross-asset wire
 * identity (`Instrument.underlying`, proto field 1, + crypto's `settlement_style`,
 * field 29) the server's `price_cross_asset` prices on the cost-of-carry seam.
 *
 * The byte-identity contract is the load-bearing invariant: with NO active
 * underlier (or an FX / metal one) the spec's output is UNCHANGED — the FX
 * perpetual frame must stay exactly what `perpetual.test.ts` already pins, no
 * `underlying`, no `settlement_style`. These cases assert both directions.
 */
import { describe, expect, it } from "vitest";

import { perpetualSpec, type PerpetualInputs } from "../../src/products/perpetual";
import { ALL_ASSET_CLASSES } from "../../src/products/capability";
import type { ProductBuildCtx } from "../../src/products/types";
import { perpetualInstrument, tenorYearsToTenor, underlyingPairProjection } from "../../src/data/seed";
import { instrumentToWire } from "../../src/data/wsCodec";
import type { SettlementStyle, Underlying } from "../../src/data/contract";
import * as e from "../../src/data/enums";

/** A representative EURUSD market/contract context (no active non-FX underlier). */
const FX_CTX: ProductBuildCtx = {
  pair: { base: "EUR", quote: "USD" },
  tenor: tenorYearsToTenor(0.25),
  tenorYears: 0.25,
  notionalMm: 10,
  pricingModel: "DEFAULT",
  atmForward: 1.105,
  spot: 1.1,
  pipDecimals: 4,
  today: { year: 2026, month: 6, day: 8 },
};

const EQUITY: Underlying = {
  kind: "equity",
  equity: { symbol: { ticker: "AAPL", venue: "XNAS" }, currency: "USD" },
  settlementCcy: "USD",
};
const COMMODITY: Underlying = {
  kind: "commodity",
  commodity: { symbol: { ticker: "CL", venue: "XNYM" }, currency: "USD" },
  settlementCcy: "USD",
};
const CRYPTO: Underlying = {
  kind: "digitalAsset",
  digitalAsset: { base: "BTC", quote: "USDT" },
  settlementCcy: "USDT",
};

/** A ctx whose active underlier overlays the agnostic arm. */
function ctxWith(underlying: Underlying, settlementStyle: SettlementStyle): ProductBuildCtx {
  return { ...FX_CTX, underlier: { underlying, settlementStyle } };
}

const INPUTS: PerpetualInputs = { optionType: "CALL", strike: 175 };

describe("perpetual product spec — cross-asset", () => {
  it("declares ALL asset classes applicable (the agnostic arm prices on every class)", () => {
    expect(perpetualSpec.applicableClasses).toEqual(ALL_ASSET_CLASSES);
    for (const cls of ["FX", "METAL", "EQUITY", "COMMODITY", "CRYPTO"] as const) {
      expect(perpetualSpec.applicableClasses).toContain(cls);
    }
  });

  it("carries the EQUITY underlying onto the built perpetual (right arm + identity)", () => {
    const inst = perpetualSpec.toInstrument(INPUTS, ctxWith(EQUITY, "LINEAR"));
    expect(inst.product.kind).toBe("perpetualOption");
    // The cross-asset identity: the `Underlying` arm is attached and the FX `pair`
    // is the underlying's leg-string projection (so FX-keyed surfaces stay total).
    expect(inst.underlying).toEqual(EQUITY);
    expect(inst.pair).toEqual(underlyingPairProjection(EQUITY));
    // The canonical no-expiry shape is preserved on the cross-asset arm.
    expect(inst.tenor).toBeUndefined();
    expect(inst.expiryYears).toBe(0);
    // LINEAR (the equity default) is the proto3 zero — presence-omitted.
    expect(inst.settlementStyle).toBeUndefined();
  });

  it("carries the COMMODITY underlying (LINEAR, presence-omitted settlement)", () => {
    const inst = perpetualSpec.toInstrument(INPUTS, ctxWith(COMMODITY, "LINEAR"));
    expect(inst.underlying).toEqual(COMMODITY);
    expect(inst.pair).toEqual(underlyingPairProjection(COMMODITY));
    expect(inst.settlementStyle).toBeUndefined();
  });

  it("carries the CRYPTO underlying AND its INVERSE_COIN settlement style", () => {
    const inst = perpetualSpec.toInstrument(INPUTS, ctxWith(CRYPTO, "INVERSE_COIN"));
    expect(inst.underlying).toEqual(CRYPTO);
    expect(inst.pair).toEqual(underlyingPairProjection(CRYPTO));
    expect(inst.settlementStyle).toBe("INVERSE_COIN");
  });

  it("encodes the cross-asset wire body the server's price_cross_asset decodes", () => {
    const w = instrumentToWire(perpetualSpec.toInstrument(INPUTS, ctxWith(CRYPTO, "INVERSE_COIN")));
    // The agnostic product body is unchanged — the carry rides the underlying.
    expect(w["perpetual_option"]).toEqual({
      option_type: e.optionType.toWire("CALL"),
      strike: 175,
      notional: 10e6,
    });
    // The cross-asset keys: the full Underlying oneof + the numeric settlement tag.
    expect(w["underlying"]).toEqual({
      digital_asset: { base: "BTC", quote: "USDT" },
      settlement_ccy: "USDT",
    });
    expect(w["settlement_style"]).toBe(e.settlementStyle.toWire("INVERSE_COIN"));
    // Still the tenorless no-expiry frame.
    expect(w["expiry_years"]).toBe(0);
    expect("tenor" in w).toBe(false);
  });

  it("delegates to the seed builder's overlay byte-for-byte (no duplicated wire-building)", () => {
    const viaSpec = perpetualSpec.toInstrument(INPUTS, ctxWith(CRYPTO, "INVERSE_COIN"));
    const viaSeed = perpetualInstrument(
      FX_CTX.pair,
      FX_CTX.notionalMm,
      { optionType: "CALL", strike: 175 },
      { underlying: CRYPTO, settlementStyle: "INVERSE_COIN" },
    );
    expect(viaSpec).toEqual(viaSeed);
    expect(instrumentToWire(viaSpec)).toEqual(instrumentToWire(viaSeed));
  });

  // --- the byte-identity invariant: FX / metal stay UNCHANGED ----------------

  it("is BYTE-IDENTICAL on FX whether or not an FX underlier is threaded", () => {
    const noUnderlier = perpetualSpec.toInstrument(INPUTS, FX_CTX);
    const fxUnderlier = perpetualSpec.toInstrument(
      INPUTS,
      ctxWith({ kind: "fx", fx: FX_CTX.pair, settlementCcy: FX_CTX.pair.quote }, "LINEAR"),
    );
    expect(noUnderlier.underlying).toBeUndefined();
    expect(noUnderlier.settlementStyle).toBeUndefined();
    expect(fxUnderlier).toEqual(noUnderlier);
    expect(instrumentToWire(fxUnderlier)).toEqual(instrumentToWire(noUnderlier));
  });

  it("is BYTE-IDENTICAL on a METAL underlier (a metal structures through the FX engine)", () => {
    const metal: Underlying = {
      kind: "metal",
      metal: { metal: "GOLD", quote: "USD" },
      settlementCcy: "USD",
    };
    const built = perpetualSpec.toInstrument(INPUTS, ctxWith(metal, "LINEAR"));
    // A metal underlier does NOT overlay the agnostic arm — it stays FX-native, so
    // the trader's `pair` is carried verbatim and no `underlying` key appears.
    expect(built.underlying).toBeUndefined();
    expect(built.pair).toEqual(FX_CTX.pair);
    expect(built).toEqual(perpetualSpec.toInstrument(INPUTS, FX_CTX));
  });
});
