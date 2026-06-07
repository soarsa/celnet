/**
 * wsCodec round-trip identity tests — the GUI end of the ONE `celnet.wire`
 * contract (CLAUDE.md rule 9). For every value type with both an encoder and a
 * decoder, `fromWire(toWire(x)) === x`: the snake_case / numeric-enum JSON the
 * server speaks is reconstructed losslessly on the way back in. We also pin the
 * frame (de)serialization that carries 64-bit `bigint` tokens without the
 * `JSON.parse` f64-rounding that would corrupt a click-to-trade token.
 *
 * These exercise the REAL `src/data/wsCodec.ts` + `src/data/enums.ts` through
 * their public surface with NO server and NO mocks.
 */
import { describe, expect, it } from "vitest";

import {
  attributionFromWire,
  attributionToWire,
  ccyPairFromWire,
  ccyPairToWire,
  conventionsFromWire,
  conventionsToWire,
  marketFromWire,
  marketToWire,
  parseFrame,
  serializeFrame,
  smileModelToWire,
} from "../src/data/wsCodec";
import type {
  AttributionRecord,
  CcyPair,
  Conventions,
  DeltaConvention,
  MarketContext,
  PremiumStyle,
  SmileModel,
} from "../src/data/contract";
import * as e from "../src/data/enums";
import { DEFAULT_CONVENTIONS, PAIRS } from "../src/data/seed";

describe("wsCodec — CcyPair round-trip", () => {
  it("reconstructs every seeded pair identically", () => {
    for (const { pair } of PAIRS) {
      expect(ccyPairFromWire(ccyPairToWire(pair))).toEqual(pair);
    }
  });

  it("encodes to the wire's bare base/quote shape", () => {
    const p: CcyPair = { base: "EUR", quote: "USD" };
    expect(ccyPairToWire(p)).toEqual({ base: "EUR", quote: "USD" });
  });
});

describe("wsCodec — Conventions round-trip", () => {
  it("reconstructs the default conventions identically", () => {
    expect(conventionsFromWire(conventionsToWire(DEFAULT_CONVENTIONS))).toEqual(
      DEFAULT_CONVENTIONS,
    );
  });

  it("round-trips every enum member across the whole convention space", () => {
    // Cartesian-ish sweep: vary each axis through all its members.
    const deltas: DeltaConvention[] = [
      "SPOT_UNADJUSTED",
      "FORWARD_UNADJUSTED",
      "SPOT_PREMIUM_ADJUSTED",
      "FORWARD_PREMIUM_ADJUSTED",
    ];
    const premiums: PremiumStyle[] = [
      "DOMESTIC_PIPS",
      "PERCENT_FOREIGN",
      "PERCENT_DOMESTIC",
      "FOREIGN_PIPS",
    ];
    for (const deltaConvention of deltas) {
      for (const premiumStyle of premiums) {
        const c: Conventions = {
          ...DEFAULT_CONVENTIONS,
          deltaConvention,
          premiumStyle,
        };
        expect(conventionsFromWire(conventionsToWire(c))).toEqual(c);
      }
    }
  });

  it("emits canonical snake_case keys with numeric enum tags", () => {
    const w = conventionsToWire(DEFAULT_CONVENTIONS);
    expect(Object.keys(w).sort()).toEqual([
      "atm_convention",
      "cut",
      "day_count",
      "delta_convention",
      "premium_style",
      "settlement",
    ]);
    expect(w["premium_style"]).toBe(e.premiumStyle.toWire("PERCENT_FOREIGN"));
  });
});

describe("wsCodec — MarketContext round-trip", () => {
  it("reconstructs every seeded market identically", () => {
    for (const { market } of PAIRS) {
      expect(marketFromWire(marketToWire(market))).toEqual(market);
    }
  });

  it("maps the GUI camelCase rate fields to the wire's r_dom/r_for", () => {
    const m: MarketContext = { spot: 1.0768, vol: 0.0755, rDom: 0.0432, rFor: 0.0218 };
    const w = marketToWire(m);
    expect(w).toEqual({ spot: 1.0768, vol: 0.0755, r_dom: 0.0432, r_for: 0.0218 });
    expect(marketFromWire(w)).toEqual(m);
  });
});

describe("wsCodec — AttributionRecord round-trip", () => {
  it("round-trips a full attribution chain", () => {
    const a: AttributionRecord = {
      quotedBy: { book: "FX-VOL", owner: { kind: "autoPricer", autoPricer: "edge-1" } },
      heldBy: { book: "EMEA-DESK", owner: { kind: "trader", trader: "jdoe" } },
      won: true,
      lpCount: 4,
    };
    // attributionToWire emits the bare body; attributionFromWire reads it nested
    // under an "attribution" key (its place on a quoted line) — so wrap to decode.
    expect(attributionFromWire({ attribution: attributionToWire(a) })).toEqual(a);
  });

  it("treats an empty body as honestly absent (undefined, not {})", () => {
    expect(attributionFromWire({ attribution: attributionToWire({}) })).toBeUndefined();
    expect(attributionFromWire({})).toBeUndefined();
  });

  it("preserves a partial chain (heldBy only)", () => {
    const a: AttributionRecord = { heldBy: { book: "APAC" }, lpCount: 0 };
    const round = attributionFromWire({ attribution: attributionToWire(a) });
    expect(round?.heldBy).toEqual({ book: "APAC" });
    expect(round?.lpCount).toBe(0);
    expect(round?.quotedBy).toBeUndefined();
  });
});

describe("wsCodec — frame (de)serialization", () => {
  it("round-trips a plain JSON object frame", () => {
    const frame = { kind: 1, pair: { base: "EUR", quote: "USD" }, spot: 1.0768 };
    expect(parseFrame(serializeFrame(frame))).toEqual(frame);
  });

  it("carries a 64-bit token through ser→parse without f64 rounding", () => {
    // A token beyond Number.MAX_SAFE_INTEGER would be corrupted by a naive
    // JSON.parse; serializeFrame writes the bigint bare and parseFrame requotes
    // the oversized literal so numToBigInt-style consumers recover it exactly.
    const token = 9_007_199_254_740_993n; // MAX_SAFE_INTEGER + 2
    const text = serializeFrame({ token });
    // The wire literal is a bare integer (no quotes) — JSON has no bigint.
    expect(text).toBe('{"token":9007199254740993}');
    const parsed = parseFrame(text) as { token: unknown };
    // parseFrame requotes it into a string to survive the parse losslessly.
    expect(BigInt(parsed.token as string)).toBe(token);
  });

  it("leaves small integers and non-integers untouched on parse", () => {
    const parsed = parseFrame('{"seq":42,"px":1.0768,"neg":-7}') as Record<string, unknown>;
    expect(parsed).toEqual({ seq: 42, px: 1.0768, neg: -7 });
  });

  it("does not requote digits that live inside a string value", () => {
    const parsed = parseFrame('{"label":"99999999999999999999 lots"}') as {
      label: string;
    };
    expect(parsed.label).toBe("99999999999999999999 lots");
  });
});

describe("wsCodec — SmileModel codec (proto-number alignment)", () => {
  // The array index MUST equal the proto enum number, or the GUI would mark a
  // surface under a different family than the trader picked.
  const PROTO_NUMBER: Record<SmileModel, number> = {
    MARKET_HEDGE: 0,
    STOCHASTIC_VOL: 1,
    PARAMETRIC: 2,
    PARAMETRIC_SURFACE: 3,
    EXTENDED_SURFACE: 4,
  };

  it("maps every SmileModel to its proto enum number on the wire", () => {
    for (const [model, number] of Object.entries(PROTO_NUMBER) as [SmileModel, number][]) {
      expect(smileModelToWire(model)).toBe(number);
    }
  });

  it("round-trips eSSVI (EXTENDED_SURFACE) through index 4 (the parity-break fix)", () => {
    expect(smileModelToWire("EXTENDED_SURFACE")).toBe(4);
    expect(e.smileModel.toWire("EXTENDED_SURFACE")).toBe(4);
    expect(e.smileModel.fromWire(4)).toBe("EXTENDED_SURFACE");
  });
});
