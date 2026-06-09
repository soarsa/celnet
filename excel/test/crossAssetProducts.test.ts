// Cross-asset integration — Excel client parity for the equity / commodity /
// digital-asset (crypto) underlying arms + the SettlementStyle the W1 generalized
// `Underlying` oneof + `Instrument.settlement_style` (proto field 29) put on the
// ONE wire contract. These tests prove the add-in shapes each cross-asset vanilla
// into the EXACT wire shape the server's `crates/celnet-server/src/ws/codec.rs`
// would decode — the `underlying` oneof keyed by proto field NAME
// (`fx`/`metal`/`equity`/`commodity`/`digital_asset`, oneof `ref` field numbers
// 1/3/4/5/6) with `settlement_ccy`, and `settlement_style` carried only when
// non-LINEAR (INVERSE_COIN = the coin-margined `1/S_T` convention). No pricing math
// lives in the add-in: a cell is bit-identical to the SDK/CLI.

import { describe, expect, it } from "vitest";
import {
  ShapingError,
  parseCryptoPair,
  shapeCommodityVanilla,
  shapeCryptoVanilla,
  shapeEquityVanilla,
  shapeMetal,
  shapeMetalVanilla,
  shapeSettlementStyle,
} from "../src/functions/shaping";
import { shapeVanillaInstrument } from "../src/functions/shaping";
import { instrumentToWire } from "../src/contract/wsCodec";
import * as e from "../src/contract/enums";

// ---------------------------------------------------------------------------
// vocabulary parsing — Metal, SettlementStyle, crypto pair
// ---------------------------------------------------------------------------

describe("cross-asset vocabulary parsing", () => {
  it("parses a metal from name / ISO X-code / wire member", () => {
    expect(shapeMetal("gold")).toBe("GOLD");
    expect(shapeMetal("XAU")).toBe("GOLD");
    expect(shapeMetal("xag")).toBe("SILVER");
    expect(shapeMetal("PLATINUM")).toBe("PLATINUM");
    expect(shapeMetal("XPD")).toBe("PALLADIUM");
    expect(() => shapeMetal("copper")).toThrow(ShapingError);
  });

  it("parses the settlement style with a LINEAR default + inverse aliases", () => {
    expect(shapeSettlementStyle(undefined)).toBe("LINEAR");
    expect(shapeSettlementStyle("linear")).toBe("LINEAR");
    expect(shapeSettlementStyle("INVERSE_COIN")).toBe("INVERSE_COIN");
    expect(shapeSettlementStyle("inverse")).toBe("INVERSE_COIN");
    expect(shapeSettlementStyle("coin")).toBe("INVERSE_COIN");
    expect(() => shapeSettlementStyle("garbage")).toThrow(ShapingError);
  });

  it("splits a crypto pair on a separator or a trailing known numeraire", () => {
    expect(parseCryptoPair("BTC-USD")).toEqual({ base: "BTC", quote: "USD" });
    expect(parseCryptoPair("eth/usdt")).toEqual({ base: "ETH", quote: "USDT" });
    expect(parseCryptoPair("BTCUSD")).toEqual({ base: "BTC", quote: "USD" });
    expect(parseCryptoPair("ETHUSDC")).toEqual({ base: "ETH", quote: "USDC" });
    expect(() => parseCryptoPair("NOTAPAIR")).toThrow(ShapingError);
  });

  it("pins the canonical Metal / SettlementStyle enum numbers the server decodes by", () => {
    expect(e.metal.toWire("GOLD")).toBe(0);
    expect(e.metal.toWire("SILVER")).toBe(1);
    expect(e.metal.toWire("PLATINUM")).toBe(2);
    expect(e.metal.toWire("PALLADIUM")).toBe(3);
    expect(e.settlementStyle.toWire("LINEAR")).toBe(0);
    expect(e.settlementStyle.toWire("INVERSE_COIN")).toBe(1);
    expect(e.settlementStyle.fromWire(1)).toBe("INVERSE_COIN");
  });
});

// ---------------------------------------------------------------------------
// equity underlying — proto Underlying.equity (oneof field 4)
// ---------------------------------------------------------------------------

describe("equity vanilla shaping + wire encoding", () => {
  it("shapes an equity underlying with the symbol + currency, FX pair projection", () => {
    const i = shapeEquityVanilla({
      ticker: "AAPL",
      currency: "USD",
      venue: "XNAS",
      tenor: "3M",
      strikeOrDelta: 200,
      callPut: "C",
      notional: 1e4,
    });
    expect(i.underlying).toEqual({
      kind: "equity",
      equity: { symbol: { ticker: "AAPL", venue: "XNAS" }, currency: "USD" },
      settlementCcy: "USD",
    });
    // The FX pair projection keeps the FX-keyed surfaces total.
    expect(i.pair).toEqual({ base: "AAPL", quote: "USD" });
    // LINEAR settlement ⇒ presence-omitted.
    expect(i.settlementStyle).toBeUndefined();
    expect(i.side).toBe("TWO_WAY");
  });

  it("encodes the `underlying` oneof under `equity` with the nested symbol + settlement_ccy", () => {
    const i = shapeEquityVanilla({
      ticker: "SPX",
      currency: "USD",
      tenor: "1Y",
      strikeOrDelta: "25dC",
      callPut: "C",
      notional: 1e4,
    });
    const wire = instrumentToWire(i);
    expect(wire["underlying"]).toEqual({
      equity: { symbol: { ticker: "SPX", venue: "" }, currency: "USD" },
      settlement_ccy: "USD",
    });
    // LINEAR is the proto3 zero value ⇒ no settlement_style on the wire.
    expect("settlement_style" in wire).toBe(false);
    // Exactly one product arm (vanilla) is present.
    expect("vanilla" in wire).toBe(true);
  });

  it("rejects a missing ticker / non-3-letter currency", () => {
    expect(() =>
      shapeEquityVanilla({ ticker: "", currency: "USD", tenor: "3M", strikeOrDelta: 1, callPut: "C", notional: 1 }),
    ).toThrow(ShapingError);
    expect(() =>
      shapeEquityVanilla({ ticker: "AAPL", currency: "DOLLARS", tenor: "3M", strikeOrDelta: 1, callPut: "C", notional: 1 }),
    ).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// commodity underlying — proto Underlying.commodity (oneof field 5)
// ---------------------------------------------------------------------------

describe("commodity vanilla shaping + wire encoding", () => {
  it("encodes the `underlying` oneof under `commodity` with the nested symbol", () => {
    const i = shapeCommodityVanilla({
      symbol: "BRENT",
      currency: "USD",
      tenor: "6M",
      strikeOrDelta: 85,
      callPut: "P",
      notional: 1e3,
    });
    expect(i.underlying?.kind).toBe("commodity");
    const wire = instrumentToWire(i);
    expect(wire["underlying"]).toEqual({
      commodity: { symbol: { ticker: "BRENT", venue: "" }, currency: "USD" },
      settlement_ccy: "USD",
    });
    expect("settlement_style" in wire).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// digital-asset (crypto) underlying — proto Underlying.digital_asset (oneof field 6)
// + SettlementStyle (linear vs inverse/coin-margined)
// ---------------------------------------------------------------------------

describe("crypto vanilla shaping + wire encoding", () => {
  it("shapes a LINEAR (stablecoin-margined) crypto vanilla with settlement_style omitted", () => {
    const i = shapeCryptoVanilla({
      pair: "BTC-USDT",
      tenor: "1M",
      strikeOrDelta: 70000,
      callPut: "C",
      notional: 10,
    });
    expect(i.underlying).toEqual({
      kind: "digitalAsset",
      digitalAsset: { base: "BTC", quote: "USDT" },
      settlementCcy: "USDT",
    });
    expect(i.settlementStyle).toBeUndefined();
    const wire = instrumentToWire(i);
    expect(wire["underlying"]).toEqual({
      digital_asset: { base: "BTC", quote: "USDT" },
      settlement_ccy: "USDT",
    });
    expect("settlement_style" in wire).toBe(false);
  });

  it("carries settlement_style=INVERSE_COIN for a coin-margined crypto vanilla", () => {
    const i = shapeCryptoVanilla({
      pair: "BTC-USD",
      tenor: "3M",
      strikeOrDelta: 65000,
      callPut: "P",
      notional: 5,
      settlementStyle: "INVERSE_COIN",
    });
    expect(i.settlementStyle).toBe("INVERSE_COIN");
    const wire = instrumentToWire(i);
    expect(wire["underlying"]).toEqual({
      digital_asset: { base: "BTC", quote: "USD" },
      settlement_ccy: "USD",
    });
    // INVERSE_COIN is non-default ⇒ carried as its numeric proto tag (1).
    expect(wire["settlement_style"]).toBe(e.settlementStyle.toWire("INVERSE_COIN"));
  });
});

// ---------------------------------------------------------------------------
// metal underlying — proto Underlying.metal (oneof field 3), ISO X-code projection
// ---------------------------------------------------------------------------

describe("metal vanilla shaping + wire encoding", () => {
  it("projects the metal leg onto its ISO X-code and encodes the numeric Metal tag", () => {
    const i = shapeMetalVanilla({
      metal: "gold",
      quote: "USD",
      tenor: "1Y",
      strikeOrDelta: 2400,
      callPut: "C",
      notional: 100,
    });
    // The FX pair projection uses the metal's ISO X-code (XAU) the registries key on.
    expect(i.pair).toEqual({ base: "XAU", quote: "USD" });
    const wire = instrumentToWire(i);
    expect(wire["underlying"]).toEqual({
      metal: { metal: e.metal.toWire("GOLD"), quote: "USD" },
      settlement_ccy: "USD",
    });
    expect("settlement_style" in wire).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// FX byte-identity — the absent underlying / LINEAR style keep the legacy frame
// ---------------------------------------------------------------------------

describe("FX frame byte-identity (no cross-asset fields leak)", () => {
  it("an FX instrument with no underlying / LINEAR style omits both new keys", () => {
    // A plain FX vanilla never sets `underlying`/`settlementStyle`, so the wire
    // frame is byte-identical to the contract before the cross-asset arms existed.
    const i = shapeVanillaInstrument({
      pair: "EURUSD",
      tenor: "3M",
      strikeOrDelta: "25dC",
      callPut: "C",
      notional: 1e6,
    });
    expect(i.underlying).toBeUndefined();
    expect(i.settlementStyle).toBeUndefined();
    const wire = instrumentToWire(i);
    expect("underlying" in wire).toBe(false);
    expect("settlement_style" in wire).toBe(false);
    expect(wire["pair"]).toEqual({ base: "EUR", quote: "USD" });
  });
});
