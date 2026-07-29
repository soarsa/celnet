/**
 * FI Tiering — the worked CLIENT-PRICE preview helper. Pins the indicative
 * outbound arithmetic the session-pivoted Tiering surface renders, mirroring the
 * server `celnet-tiering` price-space transform:
 *  - Flat markup H=25 price-bps on the reference sample 99.50/99.60 ⇒ 99.30/99.80.
 *  - guardrail clamping (hMax cap, spread floor).
 *  - no TIERING config (null) ⇒ the raw composite unchanged.
 *  - INVENTORY_SKEW at zero position reduces to its base flat markup (the lean is
 *    surfaced as position-dependent, never fabricated).
 *  - SCALED_SMOOTHED_SPREAD sources the spread from its own params (position-free).
 */

import { describe, expect, it } from "vitest";

import type { TieringConfig, TieringStrategy } from "../src/data/contract";
import { defaultTieringStrategy } from "../src/lib/tiering";
import { clientTwoWayFromTiering, hasPositionDependentSkew } from "../src/lib/tieringPreview";

/** The reference sample raw two-way (mid 99.55, market spread 0.10). */
const SAMPLE = { bid: 99.5, offer: 99.6 } as const;

/** A single-strategy Flat-markup config in `unit` at half-spread `h`, default guardrails. */
function flat(h: number, unit: TieringConfig["unit"] = "PRICE_BPS"): TieringConfig {
  const s: TieringStrategy = { ...defaultTieringStrategy("FLAT_MARKUP"), halfSpread: h };
  return {
    unit,
    strategies: [s],
    guardrails: { hMin: 0, hMax: 1, sMax: 0.5, spreadFloor: 0.01 },
    stalePolicy: "SUPPRESS",
  };
}

describe("clientTwoWayFromTiering — flat markup", () => {
  it("Flat H=25 price-bps on 99.50/99.60 ⇒ client 99.30/99.80 (the reference example)", () => {
    const out = clientTwoWayFromTiering(flat(25), SAMPLE.bid, SAMPLE.offer);
    expect(out.bid).toBeCloseTo(99.3, 10);
    expect(out.offer).toBeCloseTo(99.8, 10);
  });

  it("Flat H in PRICE_POINTS is used as-is (H=0.25 ⇒ 99.30/99.80)", () => {
    const out = clientTwoWayFromTiering(flat(0.25, "PRICE_POINTS"), SAMPLE.bid, SAMPLE.offer);
    expect(out.bid).toBeCloseTo(99.3, 10);
    expect(out.offer).toBeCloseTo(99.8, 10);
  });
});

describe("clientTwoWayFromTiering — guardrails", () => {
  it("clamps the half-spread down to hMax", () => {
    // 60 price-bps = 0.60 points, but hMax caps the half-spread at 0.10.
    const cfg = flat(60);
    cfg.guardrails = { hMin: 0, hMax: 0.1, sMax: 0.5, spreadFloor: 0.01 };
    const out = clientTwoWayFromTiering(cfg, SAMPLE.bid, SAMPLE.offer);
    expect(out.bid).toBeCloseTo(99.45, 10);
    expect(out.offer).toBeCloseTo(99.65, 10);
  });

  it("enforces the spread floor when the composed half-spread is too tight", () => {
    // Zero markup ⇒ half 0; spread_floor 0.02 forces offer − bid ≥ 0.02.
    const cfg = flat(0);
    cfg.guardrails = { hMin: 0, hMax: 1, sMax: 0.5, spreadFloor: 0.02 };
    const out = clientTwoWayFromTiering(cfg, SAMPLE.bid, SAMPLE.offer);
    expect(out.offer - out.bid).toBeCloseTo(0.02, 10);
    expect(out.bid).toBeCloseTo(99.54, 10);
    expect(out.offer).toBeCloseTo(99.56, 10);
  });
});

describe("clientTwoWayFromTiering — no tiering", () => {
  it("returns the raw composite unchanged when the config is null", () => {
    expect(clientTwoWayFromTiering(null, SAMPLE.bid, SAMPLE.offer)).toEqual({
      bid: 99.5,
      offer: 99.6,
    });
  });
});

describe("clientTwoWayFromTiering — inventory skew (zero position)", () => {
  it("reduces to the base flat markup and is flagged position-dependent", () => {
    const skew: TieringStrategy = {
      ...defaultTieringStrategy("INVENTORY_SKEW"),
      halfSpread: 25,
      kappa: 0.5,
      sMax: 0.5,
    };
    const cfg: TieringConfig = {
      unit: "PRICE_BPS",
      strategies: [skew],
      guardrails: { hMin: 0, hMax: 1, sMax: 0.5, spreadFloor: 0.01 },
      stalePolicy: "SUPPRESS",
    };
    const out = clientTwoWayFromTiering(cfg, SAMPLE.bid, SAMPLE.offer);
    // Symmetric around mid (no lean fabricated): identical to Flat 25.
    expect(out.bid).toBeCloseTo(99.3, 10);
    expect(out.offer).toBeCloseTo(99.8, 10);
    expect(hasPositionDependentSkew(cfg)).toBe(true);
    expect(hasPositionDependentSkew(flat(25))).toBe(false);
    expect(hasPositionDependentSkew(null)).toBe(false);
  });
});

describe("clientTwoWayFromTiering — scaled smoothed spread", () => {
  it("sources the output spread from its own params (position-free)", () => {
    // rawSpread 0.10, expected 0.08, maxDivergence 0.04 ⇒ D=0.02 ≤ dead-band ⇒ P=0.
    // output = min(m=0.8, c=0.20·(1+0)) = 0.20 ⇒ half 0.10 ⇒ 99.45/99.65.
    const scaled: TieringStrategy = {
      ...defaultTieringStrategy("SCALED_SMOOTHED_SPREAD"),
      smoothingWeight: 0.5,
      expectedSpread: 0.08,
      maxDivergence: 0.04,
      coreSpread: 0.2,
      maxOutputSpread: 0.8,
      spreadScaleFactor: 1.2,
    };
    const cfg: TieringConfig = {
      unit: "PRICE_POINTS",
      strategies: [scaled],
      guardrails: { hMin: 0, hMax: 1, sMax: 0.5, spreadFloor: 0.01 },
      stalePolicy: "SUPPRESS",
    };
    const out = clientTwoWayFromTiering(cfg, SAMPLE.bid, SAMPLE.offer);
    expect(out.bid).toBeCloseTo(99.45, 10);
    expect(out.offer).toBeCloseTo(99.65, 10);
    expect(hasPositionDependentSkew(cfg)).toBe(false);
  });
});
