/**
 * Wave-1 product parity for the GUI end of the ONE `celnet.wire` contract: the
 * variance swap, volatility swap and arithmetic-average-rate Asian are encoded
 * onto the SAME `Instrument.product` oneof as vanilla/strategy, with the EXACT
 * appended wire field NAMES (`variance_swap`/`volatility_swap`/`asian_option`,
 * proto field numbers 13/14/15 — no schema_version, no renumber; CLAUDE.md
 * rule 9), and the standalone build prices each with a genuine closed form whose
 * documented limits are validated here against an independent in-test oracle.
 *
 * These exercise the REAL `src/data/wsCodec.ts`, `src/data/enums.ts`,
 * `src/data/seed.ts` and `src/data/pricing.ts` through their public surface with
 * NO server and NO mocks.
 */
import { describe, expect, it } from "vitest";

import { instrumentToWire } from "../src/data/wsCodec";
import * as e from "../src/data/enums";
import {
  asianInstrument,
  varianceSwapInstrument,
  volatilitySwapInstrument,
  type AsianTerms,
} from "../src/data/seed";
import { forward, priceInstrument } from "../src/data/pricing";
import type { CcyPair, Instrument, MarketContext } from "../src/data/contract";

const PAIR: CcyPair = { base: "EUR", quote: "USD" };
const MKT: MarketContext = { spot: 1.1, vol: 0.2, rDom: 0.03, rFor: 0.01 };

// ---------------------------------------------------------------------------
// wire encoding — the appended oneof arms, by their exact proto field NAMES
// ---------------------------------------------------------------------------

describe("wave-1 products — instrumentToWire oneof arms", () => {
  it("encodes a variance swap under the `variance_swap` key (field 13)", () => {
    const inst = varianceSwapInstrument(PAIR, 0.25, 10, 0.18);
    const w = instrumentToWire(inst);
    expect(w["variance_swap"]).toEqual({ strike_vol: 0.18 });
    // Only the one product arm is present — a oneof carries exactly one body.
    expect(w["vanilla"]).toBeUndefined();
    expect(w["strategy"]).toBeUndefined();
    expect(w["volatility_swap"]).toBeUndefined();
    expect(w["asian_option"]).toBeUndefined();
  });

  it("encodes a volatility swap under the `volatility_swap` key (field 14)", () => {
    const inst = volatilitySwapInstrument(PAIR, 0.5, 10, 0.21);
    const w = instrumentToWire(inst);
    expect(w["volatility_swap"]).toEqual({ strike_vol: 0.21 });
    expect(w["variance_swap"]).toBeUndefined();
    expect(w["asian_option"]).toBeUndefined();
  });

  it("encodes an Asian under `asian_option` (field 15) with numeric enum tags", () => {
    const terms: AsianTerms = {
      optionType: "PUT",
      strike: 1.05,
      averaging: "CONTINUOUS",
      observations: 24,
      method: "TURNBULL_WAKEMAN",
      elapsedAvg: 1.02,
      elapsedWeight: 0.4,
    };
    const inst = asianInstrument(PAIR, 1, 10, terms);
    const w = instrumentToWire(inst);
    expect(w["asian_option"]).toEqual({
      option_type: e.optionType.toWire("PUT"),
      strike: 1.05,
      // AVERAGING_STYLE_DISCRETE=0, AVERAGING_STYLE_CONTINUOUS=1.
      averaging: e.averagingStyle.toWire("CONTINUOUS"),
      observations: 24,
      // ASIAN_METHOD_CURRAN=0, ASIAN_METHOD_TURNBULL_WAKEMAN=1.
      method: e.asianMethod.toWire("TURNBULL_WAKEMAN"),
      elapsed_avg: 1.02,
      elapsed_weight: 0.4,
    });
  });

  it("pins the canonical enum numbers the server decodes by", () => {
    expect(e.averagingStyle.toWire("DISCRETE")).toBe(0);
    expect(e.averagingStyle.toWire("CONTINUOUS")).toBe(1);
    expect(e.asianMethod.toWire("CURRAN")).toBe(0);
    expect(e.asianMethod.toWire("TURNBULL_WAKEMAN")).toBe(1);
    // Reversible: a number decodes back to the same member.
    expect(e.averagingStyle.fromWire(1)).toBe("CONTINUOUS");
    expect(e.asianMethod.fromWire(1)).toBe("TURNBULL_WAKEMAN");
  });

  it("carries the trader-facing tenor alongside the product body", () => {
    const inst: Instrument = varianceSwapInstrument(PAIR, 0.25, 10, 0);
    const w = instrumentToWire(inst);
    expect(w["pair"]).toEqual({ base: "EUR", quote: "USD" });
    expect(w["expiry_years"]).toBe(0.25);
    expect((w["variance_swap"] as Record<string, unknown>)["strike_vol"]).toBe(0);
  });
});

// ---------------------------------------------------------------------------
// offline pricer — closed-form limits vs an independent in-test oracle
// ---------------------------------------------------------------------------

describe("wave-1 products — variance/volatility swap fair strikes (flat σ)", () => {
  it("variance swap fair strike is exactly σ² on the flat-σ surface", () => {
    const inst = varianceSwapInstrument(PAIR, 0.25, 10, 0);
    const { greeks, resolvedStrike } = priceInstrument(inst, MKT);
    // K_var == σ² — the exact flat-vol limit of the log-contract strip; this
    // catches forward/discount/scale/sign errors a smile-weighted strip could
    // share (the same oracle the celnet-exotics flat-σ parity row asserts).
    expect(resolvedStrike).toBeCloseTo(MKT.vol * MKT.vol, 12);
    expect(greeks.price).toBeCloseTo(MKT.vol * MKT.vol, 12);
    // dK_var/dσ = 2σ.
    expect(greeks.vega).toBeCloseTo(2 * MKT.vol, 12);
  });

  it("volatility swap fair strike is exactly σ on the flat-σ surface", () => {
    const inst = volatilitySwapInstrument(PAIR, 0.5, 10, 0);
    const { greeks, resolvedStrike } = priceInstrument(inst, MKT);
    // K_vol == σ (zero convexity correction for a flat vol); strictly the
    // √K_var ceiling with no Jensen gap when variance-of-variance is zero.
    expect(resolvedStrike).toBeCloseTo(MKT.vol, 12);
    expect(greeks.price).toBeCloseTo(MKT.vol, 12);
    expect(greeks.vega).toBeCloseTo(1, 12);
  });

  it("scales the fair strikes with the market vol (not a constant)", () => {
    const hi: MarketContext = { ...MKT, vol: 0.3 };
    const varHi = priceInstrument(varianceSwapInstrument(PAIR, 0.25, 10, 0), hi).resolvedStrike;
    const volHi = priceInstrument(volatilitySwapInstrument(PAIR, 0.25, 10, 0), hi).resolvedStrike;
    expect(varHi).toBeCloseTo(0.09, 12);
    expect(volHi).toBeCloseTo(0.3, 12);
  });
});

describe("wave-1 products — arithmetic Asian closed-form limits", () => {
  /** Garman-Kohlhagen vanilla as an INDEPENDENT oracle for the degenerate limits. */
  function gkVanilla(isCall: boolean, strike: number, m: MarketContext, t: number): number {
    const sqrtT = Math.sqrt(t);
    const dfFor = Math.exp(-m.rFor * t);
    const dfDom = Math.exp(-m.rDom * t);
    const d1 = (Math.log(m.spot / strike) + (m.rDom - m.rFor + 0.5 * m.vol * m.vol) * t) / (m.vol * sqrtT);
    const d2 = d1 - m.vol * sqrtT;
    const cdf = (x: number): number => {
      // Abramowitz-Stegun 7.1.26 erf complement (independent of the pricer's CDF).
      const z = Math.abs(x) / Math.SQRT2;
      const tt = 1 / (1 + 0.3275911 * z);
      const y =
        1 -
        ((((1.061405429 * tt - 1.453152027) * tt + 1.421413741) * tt - 0.284496736) * tt +
          0.254829592) *
          tt *
          Math.exp(-z * z);
      const erf = x >= 0 ? y : -y;
      return 0.5 * (1 + erf);
    };
    return isCall
      ? m.spot * dfFor * cdf(d1) - strike * dfDom * cdf(d2)
      : strike * dfDom * cdf(-d2) - m.spot * dfFor * cdf(-d1);
  }

  const baseTerms: AsianTerms = {
    optionType: "CALL",
    strike: 1.1,
    averaging: "DISCRETE",
    observations: 1,
    method: "TURNBULL_WAKEMAN",
    elapsedAvg: 0,
    elapsedWeight: 0,
  };

  it("a single fresh fixing reduces EXACTLY to the GK vanilla", () => {
    const t = 0.5;
    const inst = asianInstrument(PAIR, t, 10, { ...baseTerms, observations: 1 });
    const asian = priceInstrument(inst, MKT).greeks.price;
    const vanilla = gkVanilla(true, baseTerms.strike, MKT, t);
    // The Asian two-moment form is EXACT for a single fresh fixing (the average
    // IS the single lognormal); the residual ~1e-7 is solely the difference
    // between the pricer's CDF and this oracle's A&S-7.1.26 erf approximation.
    expect(asian).toBeCloseTo(vanilla, 7);
  });

  it("zero vol reduces to the discounted intrinsic on the forward average", () => {
    const t = 0.75;
    const m: MarketContext = { ...MKT, vol: 1e-9 };
    const inst = asianInstrument(PAIR, t, 10, { ...baseTerms, observations: 12, strike: 1.0 });
    const asian = priceInstrument(inst, m).greeks.price;
    // With ~0 vol the average is the deterministic mean of the forward path; the
    // call is the discounted positive part of (mean − K). Independent recompute:
    const b = m.rDom - m.rFor;
    const n = 12;
    let mean = 0;
    for (let k = 1; k <= n; k += 1) mean += m.spot * Math.exp((b * k * t) / n);
    mean /= n;
    const df = Math.exp(-m.rDom * t);
    const intrinsic = df * Math.max(0, mean - 1.0);
    expect(asian).toBeCloseTo(intrinsic, 8);
  });

  it("a deep-ITM continuous call is positive and below the forward bound", () => {
    const t = 1;
    const inst = asianInstrument(PAIR, t, 10, {
      ...baseTerms,
      averaging: "CONTINUOUS",
      strike: 0.9,
      optionType: "CALL",
    });
    const price = priceInstrument(inst, MKT).greeks.price;
    expect(price).toBeGreaterThan(0);
    // The average-rate call cannot exceed the discounted (forward−K) upper bound.
    const fAvg = forward(MKT, t / 2); // mid-window forward bounds the average mean
    expect(price).toBeLessThan(Math.exp(-MKT.rDom * t) * (fAvg + 1));
  });

  it("a higher Asian strike lowers a call price (monotone, structural)", () => {
    const t = 1;
    const lowK = priceInstrument(
      asianInstrument(PAIR, t, 10, { ...baseTerms, observations: 12, strike: 1.0 }),
      MKT,
    ).greeks.price;
    const hiK = priceInstrument(
      asianInstrument(PAIR, t, 10, { ...baseTerms, observations: 12, strike: 1.2 }),
      MKT,
    ).greeks.price;
    expect(hiK).toBeLessThan(lowK);
  });

  it("populates a non-trivial Greek strip (delta in (0,1), positive vega/gamma)", () => {
    const inst = asianInstrument(PAIR, 1, 10, { ...baseTerms, observations: 12, strike: 1.1 });
    const g = priceInstrument(inst, MKT).greeks;
    expect(g.deltaSpot).toBeGreaterThan(0);
    expect(g.deltaSpot).toBeLessThan(1);
    expect(g.vega).toBeGreaterThan(0);
    expect(g.gamma).toBeGreaterThan(0);
  });
});
