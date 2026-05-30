import { describe, expect, it } from "vitest";
import {
  DEFAULT_CONVENTIONS,
  GREEK_ROWS,
  ShapingError,
  conventionFooter,
  formatGreeksSpill,
  formatRfqSpill,
  formatSmileSpill,
  formatSurfaceCubeSpill,
  parseOptionType,
  parsePair,
  parseStrikeOrDelta,
  parseTenor,
  shapeVanillaInstrument,
  subscriptionKey,
} from "../src/functions/shaping";
import type { Greeks } from "../src/contract/contract";

describe("argument parsing", () => {
  it("parses pair in both forms, case-insensitively", () => {
    expect(parsePair("EURUSD")).toEqual({ base: "EUR", quote: "USD" });
    expect(parsePair("eur/usd")).toEqual({ base: "EUR", quote: "USD" });
  });
  it("rejects a malformed pair", () => {
    expect(() => parsePair("EUR")).toThrow(ShapingError);
    expect(() => parsePair("EURUSDX")).toThrow(ShapingError);
  });

  it("parses tenors to contract Tenor + expiry years", () => {
    expect(parseTenor("1Y")).toEqual({ tenor: { unit: "YEARS", count: 1 }, expiryYears: 1 });
    expect(parseTenor("3M").tenor).toEqual({ unit: "MONTHS", count: 3 });
    expect(parseTenor("3M").expiryYears).toBeCloseTo(0.25, 12);
    expect(parseTenor("2W").tenor).toEqual({ unit: "WEEKS", count: 2 });
    expect(parseTenor("ON").tenor).toEqual({ unit: "OVERNIGHT", count: 1 });
    expect(parseTenor("O/N").tenor.unit).toBe("OVERNIGHT");
  });
  it("rejects a malformed tenor", () => {
    expect(() => parseTenor("1X")).toThrow(ShapingError);
    expect(() => parseTenor("0M")).toThrow(ShapingError);
  });

  it("parses C/P", () => {
    expect(parseOptionType("C")).toBe("CALL");
    expect(parseOptionType("put")).toBe("PUT");
    expect(() => parseOptionType("X")).toThrow(ShapingError);
  });

  it("parses strike vs delta with correct signs", () => {
    expect(parseStrikeOrDelta(1.12)).toEqual({ kind: "strike", strike: 1.12 });
    expect(parseStrikeOrDelta("1.12")).toEqual({ kind: "strike", strike: 1.12 });
    // Put delta is negative, call delta positive (matches SmilePoint.delta sign).
    expect(parseStrikeOrDelta("25dP")).toEqual({ kind: "delta", delta: -0.25 });
    expect(parseStrikeOrDelta("10dC")).toEqual({ kind: "delta", delta: 0.1 });
    expect(parseStrikeOrDelta("ATM")).toEqual({ kind: "delta", delta: 0 });
    expect(parseStrikeOrDelta("DNS")).toEqual({ kind: "delta", delta: 0 });
  });
  it("rejects an out-of-range delta", () => {
    expect(() => parseStrikeOrDelta("0dC")).toThrow(ShapingError);
    expect(() => parseStrikeOrDelta("150dC")).toThrow(ShapingError);
    expect(() => parseStrikeOrDelta("-1")).toThrow(ShapingError);
  });
});

describe("instrument shaping", () => {
  it("shapes a two-way vanilla instrument in the contract shape", () => {
    const i = shapeVanillaInstrument({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.12,
      callPut: "C",
      notional: 1_000_000,
    });
    expect(i.pair).toEqual({ base: "EUR", quote: "USD" });
    expect(i.tenor).toEqual({ unit: "YEARS", count: 1 });
    expect(i.expiryYears).toBe(1);
    expect(i.side).toBe("TWO_WAY");
    expect(i.quantity).toEqual({ notional: 1_000_000, baseCcy: true });
    expect(i.product).toEqual({
      kind: "vanilla",
      vanilla: { optionType: "CALL", strike: { kind: "strike", strike: 1.12 } },
    });
  });
  it("rejects a non-positive notional", () => {
    expect(() =>
      shapeVanillaInstrument({ pair: "EURUSD", tenor: "1Y", strikeOrDelta: 1.1, callPut: "C", notional: 0 }),
    ).toThrow(ShapingError);
  });
});

const SAMPLE_GREEKS: Greeks = {
  price: 0.04,
  deltaSpot: 0.51,
  deltaForward: 0.5,
  gamma: 2.1,
  vega: 0.0039,
  theta: -0.00012,
  rhoDom: 0.004,
  rhoFor: -0.005,
  vanna: 0.001,
  volga: 0.002,
  charm: 0.0001,
  speed: -0.3,
  zomma: 0.05,
  color: -0.0002,
};

describe("dynamic-array formatting", () => {
  it("spills exactly the 13 risk Greeks in contract order, plus a footer", () => {
    const m = formatGreeksSpill(SAMPLE_GREEKS, DEFAULT_CONVENTIONS, 1284n, 1_700_000_000_000_000_000n);
    expect(m.length).toBe(GREEK_ROWS.length + 1); // 13 Greeks + 1 footer
    expect(m[0]).toEqual(["delta_spot", 0.51]);
    expect(m[12]).toEqual(["color", -0.0002]);
    // Footer carries the convention transparency.
    expect(String(m[13]?.[0])).toContain("surface v1284");
    expect(String(m[13]?.[0])).toContain("SPOT_UNADJUSTED");
    // Greek price is NOT in the GREEKS spill (it is CELNET.PRICE's job).
    expect(m.flat()).not.toContain(SAMPLE_GREEKS.price);
  });

  it("conventionFooter renders live vs versioned surface", () => {
    expect(conventionFooter(DEFAULT_CONVENTIONS, undefined, 0n)).toContain("surface live");
    expect(conventionFooter(DEFAULT_CONVENTIONS, 7n, 0n)).toContain("surface v7");
  });

  it("spills a smile row sorted by signed delta with arb + convention footer", () => {
    const m = formatSmileSpill(
      [
        { delta: 0.25, vol: 0.101 },
        { delta: -0.25, vol: 0.108 },
        { delta: 0, vol: 0.1 },
      ],
      true,
      DEFAULT_CONVENTIONS,
      9n,
      0n,
    );
    expect(m[0]).toEqual(["delta", -0.25, 0, 0.25]);
    expect(m[1]).toEqual(["vol", 0.108, 0.1, 0.101]);
    expect(String(m[2]?.[0])).toContain("arb-free");
  });

  it("flags arbitrage in the smile footer", () => {
    const m = formatSmileSpill([{ delta: 0, vol: 0.1 }], false, DEFAULT_CONVENTIONS, 9n, 0n);
    expect(String(m[2]?.[0])).toContain("ARB!");
  });

  it("spills a surface cube as tenor x delta-pillar with blanks for gaps", () => {
    const m = formatSurfaceCubeSpill(
      [
        { tenorYears: 0.25, points: [{ delta: -0.25, vol: 0.11 }, { delta: 0, vol: 0.1 }] },
        { tenorYears: 1, points: [{ delta: 0, vol: 0.105 }, { delta: 0.25, vol: 0.102 }] },
      ],
      DEFAULT_CONVENTIONS,
      undefined,
      0n,
    );
    expect(m[0]).toEqual(["tenor\\delta", -0.25, 0, 0.25]);
    // First tenor has no +0.25 pillar -> blank.
    expect(m[1]).toEqual([0.25, 0.11, 0.1, ""]);
    // Second tenor has no -0.25 pillar -> blank.
    expect(m[2]).toEqual([1, "", 0.105, 0.102]);
  });

  it("spills an RFQ as [bid, offer, quoteId, validUntil] + footer; ids as strings", () => {
    const m = formatRfqSpill({
      bid: 0.039,
      offer: 0.041,
      quoteId: 123n,
      validUntilNanos: 1_700_000_000_000_000_000n,
      conventions: DEFAULT_CONVENTIONS,
      surfaceVersion: 5n,
      epochNanos: 0n,
    });
    expect(m[0]?.[0]).toBe(0.039);
    expect(m[0]?.[1]).toBe(0.041);
    expect(m[0]?.[2]).toBe("123"); // bigint id rendered as string (no precision loss)
    expect(String(m[1]?.[0])).toContain("surface v5");
  });
});

describe("subscription coalescing key", () => {
  it("maps identical arguments to the same key regardless of source whitespace", () => {
    const a = shapeVanillaInstrument({ pair: "EURUSD", tenor: "1Y", strikeOrDelta: 1.12, callPut: "C", notional: 1e6 });
    const b = shapeVanillaInstrument({ pair: "eur/usd", tenor: "1y", strikeOrDelta: "1.12", callPut: "call", notional: 1e6 });
    expect(subscriptionKey(a, DEFAULT_CONVENTIONS)).toBe(subscriptionKey(b, DEFAULT_CONVENTIONS));
  });
  it("distinguishes different structures", () => {
    const a = shapeVanillaInstrument({ pair: "EURUSD", tenor: "1Y", strikeOrDelta: 1.12, callPut: "C", notional: 1e6 });
    const b = shapeVanillaInstrument({ pair: "EURUSD", tenor: "1Y", strikeOrDelta: 1.12, callPut: "P", notional: 1e6 });
    expect(subscriptionKey(a, DEFAULT_CONVENTIONS)).not.toBe(subscriptionKey(b, DEFAULT_CONVENTIONS));
  });
});
