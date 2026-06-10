// Excel client parity for the two new payoff shapes the Rust slice put on the
// ONE wire contract: the perpetual (no-expiry) American option (proto arm 30)
// and the option on a listed future (proto arm 31). These tests prove the
// add-in shapes each product into the EXACT wire frame the server's
// `crates/celnet-server/src/ws/codec.rs` decodes — `perpetual_option` is the one
// TENORLESS instrument shape (`expiry_years = 0` exactly, NO `tenor` key, the
// shape `convert::validate_perpetual_terms` guards); `listed_future_option`
// nests the future's contract identity (`future_symbol`: ticker + venue MIC),
// the future's own expiry (which must outlive the option's) and the numeric
// `margining` tag. The hand-built expected frames below mirror the server
// codec's own round-trip tests field-for-field. No pricing math lives in the
// add-in: a cell is bit-identical to the SDK/CLI (whose own gates reconcile the
// server pricer to the closed forms vs independent oracles).

import { describe, expect, it } from "vitest";

import {
  ShapingError,
  parseFutureSymbol,
  parseMargining,
  shapeListedFutureOption,
  shapePerpetual,
  subscriptionKey,
  shapeVanillaInstrument,
  DEFAULT_CONVENTIONS,
} from "../src/functions/shaping";
import {
  decodeInstrumentToken,
  encodeInstrumentToken,
  instrumentLabel,
  shapeSpecInstrument,
} from "../src/functions/instrumentSpec";
import { instrumentToWire } from "../src/contract/wsCodec";
import { instrumentFromWire } from "../src/contract/instrumentCodec";
import * as e from "../src/contract/enums";

// ---------------------------------------------------------------------------
// argument parsing
// ---------------------------------------------------------------------------

describe("margining / future-symbol parsing", () => {
  it("parses the margining convention with an EQUITY_STYLE default and rejects garbage", () => {
    expect(parseMargining(undefined)).toBe("EQUITY_STYLE");
    expect(parseMargining("equity")).toBe("EQUITY_STYLE");
    expect(parseMargining("EQUITY_STYLE")).toBe("EQUITY_STYLE");
    expect(parseMargining("upfront")).toBe("EQUITY_STYLE");
    expect(parseMargining("futures")).toBe("FUTURES_STYLE");
    expect(parseMargining("FUTURES_STYLE")).toBe("FUTURES_STYLE");
    expect(parseMargining("daily")).toBe("FUTURES_STYLE");
    expect(() => parseMargining("monthly")).toThrow(ShapingError);
  });

  it("parses TICKER[@VENUE] future symbols and rejects an empty ticker / bad venue", () => {
    expect(parseFutureSymbol("CL@XNYM")).toEqual({ ticker: "CL", venue: "XNYM" });
    expect(parseFutureSymbol("brn-dec26@ifeu")).toEqual({ ticker: "BRN-DEC26", venue: "IFEU" });
    // An omitted venue is the proto `Symbol` zero value (unambiguous contract).
    expect(parseFutureSymbol("ES")).toEqual({ ticker: "ES", venue: "" });
    expect(() => parseFutureSymbol("@XCME")).toThrow(ShapingError);
    expect(() => parseFutureSymbol("ES@X CME")).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// perpetual (proto arm 30) — shaping → wire encoding
// ---------------------------------------------------------------------------

describe("perpetual option shaping + wire encoding", () => {
  it("shapes the contract's canonical tenorless form (expiryYears = 0, no tenor)", () => {
    const i = shapePerpetual({ pair: "EURUSD", strike: 1.05, callPut: "P", notional: 1e7 });
    expect(i.pair).toEqual({ base: "EUR", quote: "USD" });
    expect(i.tenor).toBeUndefined();
    // The exact proto3 zero the shared validity seam demands — never a tiny dt.
    expect(Object.is(i.expiryYears, 0)).toBe(true);
    expect(i.quantity).toEqual({ notional: 1e7, baseCcy: true });
    expect(i.side).toBe("TWO_WAY");
    // The product notional mirrors the one quantity (the SDK's builder shape).
    expect(i.product).toEqual({
      kind: "perpetualOption",
      perpetualOption: { optionType: "PUT", strike: 1.05, notional: 1e7 },
    });
  });

  it("encodes the oneof under `perpetual_option` (proto field 30) as the HAND-BUILT server frame", () => {
    const wire = instrumentToWire(
      shapePerpetual({ pair: "EURUSD", strike: 1.05, callPut: "P", notional: 1e7 }),
    );
    // Field-for-field the frame the server codec's own round-trip test decodes.
    expect(wire).toEqual({
      pair: { base: "EUR", quote: "USD" },
      expiry_years: 0,
      quantity: { notional: 1e7, base_ccy: true },
      side: e.side.toWire("TWO_WAY"),
      perpetual_option: {
        option_type: e.optionType.toWire("PUT"), // PUT = 1
        strike: 1.05,
        notional: 1e7,
      },
    });
    // A perpetual is TENORLESS on the wire — the key is ABSENT, not null/zeroed.
    expect("tenor" in wire).toBe(false);
    // Exactly one product arm is present (oneof discipline).
    expect("vanilla" in wire).toBe(false);
    expect("american" in wire).toBe(false);
    expect("listed_future_option" in wire).toBe(false);
  });

  it("decodes the server's wire frame back to the typed instrument (codec inverse)", () => {
    const decoded = instrumentFromWire({
      pair: { base: "EUR", quote: "USD" },
      expiry_years: 0,
      quantity: { notional: 1e7, base_ccy: true },
      side: 2,
      perpetual_option: { option_type: 1, strike: 1.05, notional: 1e7 },
    });
    expect(decoded.tenor).toBeUndefined();
    expect(decoded.expiryYears).toBe(0);
    expect(decoded.product).toEqual({
      kind: "perpetualOption",
      perpetualOption: { optionType: "PUT", strike: 1.05, notional: 1e7 },
    });
  });

  it("rejects a non-positive strike / notional with a typed error", () => {
    expect(() => shapePerpetual({ pair: "EURUSD", strike: 0, callPut: "C", notional: 1 })).toThrow(
      ShapingError,
    );
    expect(() => shapePerpetual({ pair: "EURUSD", strike: NaN, callPut: "C", notional: 1 })).toThrow(
      ShapingError,
    );
    expect(() => shapePerpetual({ pair: "EURUSD", strike: 1.1, callPut: "C", notional: 0 })).toThrow(
      ShapingError,
    );
  });
});

// ---------------------------------------------------------------------------
// listed-future option (proto arm 31) — shaping → wire encoding
// ---------------------------------------------------------------------------

describe("listed-future option shaping + wire encoding", () => {
  it("encodes the oneof under `listed_future_option` (proto field 31) as the HAND-BUILT server frame", () => {
    const i = shapeListedFutureOption({
      pair: { base: "BRENT", quote: "USD" },
      tenor: "6M",
      strike: 85,
      callPut: "C",
      notional: 1000,
      futureSymbol: "BRN-DEC26@IFEU",
      futureExpiry: 0.55,
      margining: "FUTURES_STYLE",
    });
    const wire = instrumentToWire(i);
    // Field-for-field the body the server codec's own round-trip test decodes.
    expect(wire["listed_future_option"]).toEqual({
      future_symbol: { ticker: "BRN-DEC26", venue: "IFEU" },
      future_expiry_years: 0.55,
      option_type: e.optionType.toWire("CALL"), // CALL = 0
      strike: 85,
      notional: 1000,
      margining: e.margining.toWire("FUTURES_STYLE"), // FUTURES_STYLE = 1
    });
    // A dated product: the tenor IS carried (only the perpetual is tenorless).
    expect(wire["tenor"]).toEqual({ unit: e.tenorUnit.toWire("MONTHS"), count: 6 });
    expect("perpetual_option" in wire).toBe(false);
    expect("vanilla" in wire).toBe(false);
  });

  it("defaults the margining to the meaningful-zero EQUITY_STYLE (always encoded, like every body enum)", () => {
    const i = shapeListedFutureOption({
      pair: { base: "CL", quote: "USD" },
      tenor: "9M",
      strike: 19,
      callPut: "C",
      notional: 1,
      futureSymbol: "CL@XNYM",
      futureExpiry: 0.75,
    });
    const wire = instrumentToWire(i);
    expect((wire["listed_future_option"] as Record<string, unknown>)["margining"]).toBe(0);
  });

  it("rejects a future that does not outlive the option (the server's term guard, typed)", () => {
    // 9M option on a future expiring at 0.5y: futureExpiry < expiryYears.
    expect(() =>
      shapeListedFutureOption({
        pair: { base: "CL", quote: "USD" },
        tenor: "9M",
        strike: 19,
        callPut: "C",
        notional: 1,
        futureSymbol: "CL@XNYM",
        futureExpiry: 0.5,
      }),
    ).toThrow(/outlive/);
    expect(() =>
      shapeListedFutureOption({
        pair: { base: "CL", quote: "USD" },
        tenor: "9M",
        strike: 19,
        callPut: "C",
        notional: 1,
        futureSymbol: "CL@XNYM",
        futureExpiry: NaN,
      }),
    ).toThrow(ShapingError);
  });

  it("rejects an unknown margining / empty future symbol / bad strike with typed errors", () => {
    const base = {
      pair: { base: "CL", quote: "USD" },
      tenor: "9M",
      strike: 19,
      callPut: "C",
      notional: 1,
      futureSymbol: "CL@XNYM",
      futureExpiry: 0.75,
    } as const;
    expect(() => shapeListedFutureOption({ ...base, margining: "WEEKLY" })).toThrow(ShapingError);
    expect(() => shapeListedFutureOption({ ...base, futureSymbol: "@XNYM" })).toThrow(ShapingError);
    expect(() => shapeListedFutureOption({ ...base, strike: -1 })).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// the polymorphic CELNET.INSTRUMENT dispatch + the opaque token
// ---------------------------------------------------------------------------

describe("INSTRUMENT family dispatch: PERPETUAL", () => {
  it("shapes from terms with NO tenor and round-trips the token losslessly", () => {
    const spec = shapeSpecInstrument({
      underlier: "EURUSD",
      product: "PERPETUAL",
      terms: [
        ["strike", 1.05],
        ["callPut", "P"],
        ["notional", 1e7],
      ],
    });
    expect(spec).toEqual(shapePerpetual({ pair: "EURUSD", strike: 1.05, callPut: "P", notional: 1e7 }));
    const token = encodeInstrumentToken(spec);
    // The token (canonical wire JSON) carries no tenor key at all.
    expect(token.includes('"tenor"')).toBe(false);
    // decode → re-encode reproduces the byte-identical token.
    expect(encodeInstrumentToken(decodeInstrumentToken(token))).toBe(token);
  });

  it("answers to the proto-arm name `perpetual_option` verbatim", () => {
    const byArm = shapeSpecInstrument({
      underlier: "USDJPY",
      product: "perpetual_option",
      terms: [
        ["strike", 150],
        ["callPut", "P"],
      ],
    });
    const byName = shapeSpecInstrument({
      underlier: "USDJPY",
      product: "PERPETUAL",
      terms: [
        ["strike", 150],
        ["callPut", "P"],
      ],
    });
    expect(encodeInstrumentToken(byArm)).toBe(encodeInstrumentToken(byName));
  });

  it("rejects a tenor ARGUMENT with a typed error (a perpetual has no expiry)", () => {
    expect(() =>
      shapeSpecInstrument({
        underlier: "EURUSD",
        product: "PERPETUAL",
        terms: [
          ["strike", 1.05],
          ["callPut", "C"],
        ],
        tenor: "1Y",
      }),
    ).toThrow(/no tenor/);
  });

  it("rejects a (\"tenor\", …) TERM with a typed error — never a silent drop", () => {
    expect(() =>
      shapeSpecInstrument({
        underlier: "EURUSD",
        product: "PERPETUAL",
        terms: [
          ["strike", 1.05],
          ["callPut", "C"],
          ["tenor", "1Y"],
        ],
      }),
    ).toThrow(/no tenor/);
  });

  it("rejects an unknown term NAMING the key and the family's key set", () => {
    expect(() =>
      shapeSpecInstrument({
        underlier: "EURUSD",
        product: "PERPETUAL",
        terms: [
          ["strike", 1.05],
          ["callPut", "C"],
          ["barrier", 1.2],
        ],
      }),
    ).toThrow(/`barrier`.*strike, callPut/s);
  });

  it("overlays a crypto underlier with the inverse settlement (the inverse-perpetual desk shape)", () => {
    const spec = shapeSpecInstrument({
      underlier: "BTC/USD:inverse",
      product: "PERPETUAL",
      terms: [
        ["strike", 60000],
        ["callPut", "C"],
      ],
    });
    const wire = instrumentToWire(spec);
    expect(wire["underlying"]).toEqual({
      digital_asset: { base: "BTC", quote: "USD" },
      settlement_ccy: "USD",
    });
    expect(wire["settlement_style"]).toBe(e.settlementStyle.toWire("INVERSE_COIN"));
    expect("tenor" in wire).toBe(false);
    expect(wire["expiry_years"]).toBe(0);
    // Lossless through the token (the cross-asset + tenorless form included).
    const token = encodeInstrumentToken(spec);
    expect(encodeInstrumentToken(decodeInstrumentToken(token))).toBe(token);
  });

  it("labels a decoded perpetual honestly (perp, not a 0.00y maturity)", () => {
    const spec = shapeSpecInstrument({
      underlier: "EURUSD",
      product: "PERPETUAL",
      terms: [
        ["strike", 1.05],
        ["callPut", "C"],
      ],
    });
    expect(instrumentLabel(decodeInstrumentToken(encodeInstrumentToken(spec)))).toBe(
      "EURUSD perpetualOption perp",
    );
  });

  it("keys a perpetual subscription distinctly from a dated vanilla on the same pair", () => {
    const perp = shapePerpetual({ pair: "EURUSD", strike: 1.05, callPut: "C", notional: 1 });
    const dated = shapeVanillaInstrument({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.05,
      callPut: "C",
      notional: 1,
    });
    expect(subscriptionKey(perp, DEFAULT_CONVENTIONS)).not.toBe(
      subscriptionKey(dated, DEFAULT_CONVENTIONS),
    );
    // Identical perpetual specs coalesce onto one subscription.
    const again = shapePerpetual({ pair: "EURUSD", strike: 1.05, callPut: "C", notional: 1 });
    expect(subscriptionKey(perp, DEFAULT_CONVENTIONS)).toBe(
      subscriptionKey(again, DEFAULT_CONVENTIONS),
    );
  });
});

describe("INSTRUMENT family dispatch: FUTUREOPTION", () => {
  const TERMS: (string | number)[][] = [
    ["strike", 40],
    ["callPut", "C"],
    ["futureSymbol", "ES@XCME"],
    ["futureExpiry", 0.75],
    ["margining", "FUTURES_STYLE"],
  ];

  it("shapes from terms onto an equity-index underlier and round-trips the token losslessly", () => {
    const spec = shapeSpecInstrument({
      underlier: "ES@XCME:USD",
      product: "FUTUREOPTION",
      terms: TERMS,
      tenor: "6M",
    });
    const wire = instrumentToWire(spec);
    expect(wire["underlying"]).toEqual({
      equity: { symbol: { ticker: "ES", venue: "XCME" }, currency: "USD" },
      settlement_ccy: "USD",
    });
    expect(wire["listed_future_option"]).toEqual({
      future_symbol: { ticker: "ES", venue: "XCME" },
      future_expiry_years: 0.75,
      option_type: 0,
      strike: 40,
      notional: 1,
      margining: 1,
    });
    const token = encodeInstrumentToken(spec);
    expect(encodeInstrumentToken(decodeInstrumentToken(token))).toBe(token);
  });

  it("answers to the proto-arm name `listed_future_option` verbatim", () => {
    const byArm = shapeSpecInstrument({
      underlier: "ES@XCME:USD",
      product: "listed_future_option",
      terms: TERMS,
      tenor: "6M",
    });
    const byName = shapeSpecInstrument({
      underlier: "ES@XCME:USD",
      product: "FUTUREOPTION",
      terms: TERMS,
      tenor: "6M",
    });
    expect(encodeInstrumentToken(byArm)).toBe(encodeInstrumentToken(byName));
  });

  it("requires its terms with typed errors NAMING the missing key", () => {
    expect(() =>
      shapeSpecInstrument({
        underlier: "ES@XCME:USD",
        product: "FUTUREOPTION",
        terms: [
          ["strike", 40],
          ["callPut", "C"],
          ["futureExpiry", 0.75],
        ],
        tenor: "6M",
      }),
    ).toThrow(/`futureSymbol`/);
    expect(() =>
      shapeSpecInstrument({
        underlier: "ES@XCME:USD",
        product: "FUTUREOPTION",
        terms: [
          ["strike", 40],
          ["callPut", "C"],
          ["futureSymbol", "ES@XCME"],
        ],
        tenor: "6M",
      }),
    ).toThrow(/`futureExpiry`/);
  });

  it("requires a tenor (a listed-future OPTION is dated — only the perpetual is not)", () => {
    expect(() =>
      shapeSpecInstrument({
        underlier: "ES@XCME:USD",
        product: "FUTUREOPTION",
        terms: TERMS,
      }),
    ).toThrow(/tenor is required/);
  });
});
