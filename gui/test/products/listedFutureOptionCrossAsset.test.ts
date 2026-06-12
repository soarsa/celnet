/**
 * Cross-asset gate for the option on a listed future — proving the
 * asset-class-AGNOSTIC arm (proto `listed_future_option`, field 31) BUILDS on an
 * equity / commodity / crypto underlier and carries the correct cross-asset wire
 * identity (`Instrument.underlying`, proto field 1, + crypto's `settlement_style`,
 * field 29) the server's `price_cross_asset` prices on the cost-of-carry seam. The
 * quoted futures price embodies the carry, so the futures-measure body is unchanged;
 * the enclosing `underlying` names the class while `future_symbol` names the contract.
 *
 * The byte-identity contract is the load-bearing invariant: with NO active underlier
 * (or an FX / metal one) the spec's output is exactly what `listedFutureOption.test.ts`
 * already pins — no `underlying`, no `settlement_style`. These cases assert both.
 */
import { describe, expect, it } from "vitest";

import {
  listedFutureOptionSpec,
  type ListedFutureOptionInputs,
} from "../../src/products/listedFutureOption";
import { ALL_ASSET_CLASSES } from "../../src/products/capability";
import type { ProductBuildCtx } from "../../src/products/types";
import {
  listedFutureOptionInstrument,
  tenorYearsToTenor,
  underlyingPairProjection,
} from "../../src/data/seed";
import { instrumentToWire } from "../../src/data/wsCodec";
import type { SettlementStyle, Underlying } from "../../src/data/contract";
import * as e from "../../src/data/enums";

/** A representative EURUSD 3M market/contract context (no active non-FX underlier). */
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

function ctxWith(underlying: Underlying, settlementStyle: SettlementStyle): ProductBuildCtx {
  return { ...FX_CTX, underlier: { underlying, settlementStyle } };
}

const INPUTS: ListedFutureOptionInputs = {
  optionType: "CALL",
  strike: 175,
  futureTicker: "ESZ6",
  futureVenue: "XCME",
  futureLagYears: 0.1,
  margining: "FUTURES_STYLE",
};

describe("listed-future-option product spec — cross-asset", () => {
  it("declares ALL asset classes applicable (the agnostic arm prices on every class)", () => {
    expect(listedFutureOptionSpec.applicableClasses).toEqual(ALL_ASSET_CLASSES);
    for (const cls of ["FX", "METAL", "EQUITY", "COMMODITY", "CRYPTO"] as const) {
      expect(listedFutureOptionSpec.applicableClasses).toContain(cls);
    }
  });

  it("carries the EQUITY underlying onto the built future-option (right arm + identity)", () => {
    const inst = listedFutureOptionSpec.toInstrument(INPUTS, ctxWith(EQUITY, "LINEAR"));
    expect(inst.product.kind).toBe("listedFutureOption");
    expect(inst.underlying).toEqual(EQUITY);
    expect(inst.pair).toEqual(underlyingPairProjection(EQUITY));
    // The OPTION's tenor is stamped (the dated arm); the future outlives it.
    expect(inst.tenor).toEqual(FX_CTX.tenor);
    expect(inst.expiryYears).toBe(FX_CTX.tenorYears);
    if (inst.product.kind !== "listedFutureOption") throw new Error("expected the arm");
    expect(inst.product.listedFutureOption.futureExpiryYears).toBe(FX_CTX.tenorYears + 0.1);
    expect(inst.settlementStyle).toBeUndefined();
  });

  it("carries the COMMODITY underlying (LINEAR, presence-omitted settlement)", () => {
    const inst = listedFutureOptionSpec.toInstrument(INPUTS, ctxWith(COMMODITY, "LINEAR"));
    expect(inst.underlying).toEqual(COMMODITY);
    expect(inst.pair).toEqual(underlyingPairProjection(COMMODITY));
    expect(inst.settlementStyle).toBeUndefined();
  });

  it("carries the CRYPTO underlying AND its INVERSE_COIN settlement style", () => {
    const inst = listedFutureOptionSpec.toInstrument(INPUTS, ctxWith(CRYPTO, "INVERSE_COIN"));
    expect(inst.underlying).toEqual(CRYPTO);
    expect(inst.pair).toEqual(underlyingPairProjection(CRYPTO));
    expect(inst.settlementStyle).toBe("INVERSE_COIN");
  });

  it("encodes the cross-asset wire body the server's price_cross_asset decodes", () => {
    const w = instrumentToWire(
      listedFutureOptionSpec.toInstrument(INPUTS, ctxWith(EQUITY, "LINEAR")),
    );
    // The futures-measure body is unchanged — the carry rides the future/underlying.
    expect(w["listed_future_option"]).toEqual({
      future_symbol: { ticker: "ESZ6", venue: "XCME" },
      future_expiry_years: FX_CTX.tenorYears + 0.1,
      option_type: e.optionType.toWire("CALL"),
      strike: 175,
      notional: 10e6,
      margining: e.margining.toWire("FUTURES_STYLE"),
    });
    // The cross-asset key: the full Underlying oneof (equity LINEAR → no settlement key).
    expect(w["underlying"]).toEqual({
      equity: { symbol: { ticker: "AAPL", venue: "XNAS" }, currency: "USD" },
      settlement_ccy: "USD",
    });
    expect("settlement_style" in w).toBe(false);
  });

  it("emits the crypto settlement_style tag on the wire", () => {
    const w = instrumentToWire(
      listedFutureOptionSpec.toInstrument(INPUTS, ctxWith(CRYPTO, "INVERSE_COIN")),
    );
    expect(w["underlying"]).toEqual({
      digital_asset: { base: "BTC", quote: "USDT" },
      settlement_ccy: "USDT",
    });
    expect(w["settlement_style"]).toBe(e.settlementStyle.toWire("INVERSE_COIN"));
  });

  it("delegates to the seed builder's overlay byte-for-byte (no duplicated wire-building)", () => {
    const viaSpec = listedFutureOptionSpec.toInstrument(INPUTS, ctxWith(CRYPTO, "INVERSE_COIN"));
    const viaSeed = listedFutureOptionInstrument(
      FX_CTX.pair,
      FX_CTX.tenorYears,
      FX_CTX.notionalMm,
      {
        futureSymbol: { ticker: "ESZ6", venue: "XCME" },
        futureExpiryYears: FX_CTX.tenorYears + 0.1,
        optionType: "CALL",
        strike: 175,
        margining: "FUTURES_STYLE",
      },
      { underlying: CRYPTO, settlementStyle: "INVERSE_COIN" },
    );
    expect(viaSpec).toEqual(viaSeed);
    expect(instrumentToWire(viaSpec)).toEqual(instrumentToWire(viaSeed));
  });

  // --- the byte-identity invariant: FX / metal stay UNCHANGED ----------------

  it("is BYTE-IDENTICAL on FX whether or not an FX underlier is threaded", () => {
    const noUnderlier = listedFutureOptionSpec.toInstrument(INPUTS, FX_CTX);
    const fxUnderlier = listedFutureOptionSpec.toInstrument(
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
    const built = listedFutureOptionSpec.toInstrument(INPUTS, ctxWith(metal, "LINEAR"));
    expect(built.underlying).toBeUndefined();
    expect(built.pair).toEqual(FX_CTX.pair);
    expect(built).toEqual(listedFutureOptionSpec.toInstrument(INPUTS, FX_CTX));
  });
});
