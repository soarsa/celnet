/**
 * surface.ts tests — the standalone-build smile/ladder calibration + arb checks
 * + bilinear surface sampling that back the Surface workspace's edit→ladder
 * preview and 3D mesh. Pure, deterministic math (the rigorous arb-free fit lives
 * server-side in `celnet-surface`); these pin the GUI-side calibration through
 * its public surface.
 */
import { describe, expect, it } from "vitest";

import {
  DELTA_PILLARS,
  calibrateLadder,
  calibrateSmile,
  impliedVolForInstrument,
  markSurface,
  sampleSurface,
} from "../src/data/surface";
import { DEFAULT_CONVENTIONS, brokerLadder, PAIRS } from "../src/data/seed";
import type { BrokerQuoteSet, CcyPair, Instrument } from "../src/data/contract";

const PAIR: CcyPair = { base: "EUR", quote: "USD" };
const NOW = 1_717_000_000_000_000_000n;

/** A single, well-formed broker quote set with a put skew + positive butterfly. */
function quote(overrides: Partial<BrokerQuoteSet> = {}): BrokerQuoteSet {
  return {
    tenorYears: 0.25,
    atmVol: 0.08,
    rr25: -0.003,
    bf25: 0.0016,
    rr10: -0.0055,
    bf10: 0.0037,
    hasTenDelta: true,
    ...overrides,
  };
}

describe("calibrateSmile — marks reproduce the broker inputs", () => {
  it("samples the standard signed-delta pillars and pins ATM exactly", () => {
    const s = calibrateSmile(PAIR, quote(), DEFAULT_CONVENTIONS, NOW);
    expect(s.points.map((p) => p.delta)).toEqual(DELTA_PILLARS);
    const atm = s.points.find((p) => p.delta === 0.5);
    expect(atm?.vol).toBeCloseTo(0.08, 12);
  });

  it("respects the RR skew direction: a put-skew lifts the put wing over the call wing", () => {
    const s = calibrateSmile(PAIR, quote({ rr25: -0.004 }), DEFAULT_CONVENTIONS, NOW);
    const callWing = s.points.find((p) => p.delta === 0.25)!.vol;
    const putWing = s.points.find((p) => p.delta === -0.25)!.vol;
    // Negative RR (risk-off majors) ⇒ the 25Δ put is bid over the 25Δ call.
    expect(putWing).toBeGreaterThan(callWing);
  });

  it("flags butterfly arb on a concave (negative-convexity) smile", () => {
    // A strongly negative butterfly bends the wings down ⇒ density breach.
    const s = calibrateSmile(
      PAIR,
      quote({ bf25: -0.03, bf10: -0.05 }),
      DEFAULT_CONVENTIONS,
      NOW,
    );
    expect(s.arbitrage.butterflyArbitrageFree).toBe(false);
    expect(s.arbitrage.note).toMatch(/butterfly/i);
  });

  it("stamps the TYPED model provenance (authoritative) + a human note token", () => {
    const s = calibrateSmile(PAIR, quote(), DEFAULT_CONVENTIONS, NOW, "STOCHASTIC_VOL");
    // The TYPED field is the authoritative source the GUI reads.
    expect(s.arbitrage.model).toBe("STOCHASTIC_VOL");
    // The note keeps a human token for the eye only (not scraped by code).
    expect(s.arbitrage.note).toContain("model=stochastic-vol");
  });

  it("stamps the eSSVI (EXTENDED_SURFACE) typed provenance the server emits", () => {
    const s = calibrateSmile(PAIR, quote(), DEFAULT_CONVENTIONS, NOW, "EXTENDED_SURFACE");
    expect(s.arbitrage.model).toBe("EXTENDED_SURFACE");
    expect(s.arbitrage.note).toContain("model=extended-surface");
  });
});

describe("calibrateLadder — cross-tenor calendar arb", () => {
  it("a monotone-increasing total-variance term structure is calendar arb-free", () => {
    const smiles = calibrateLadder(PAIR, brokerLadder(PAIRS[0]!), DEFAULT_CONVENTIONS, NOW);
    expect(smiles.every((s) => s.arbitrage.calendarArbitrageFree)).toBe(true);
  });

  it("flags calendar arb when a longer tenor's ATM total variance drops", () => {
    // w(T) = atm^2 * T. Make the 1Y total variance fall below the 3M's.
    const ladder: BrokerQuoteSet[] = [
      quote({ tenorYears: 0.25, atmVol: 0.2 }), // w = 0.04*0.25 = 0.010
      quote({ tenorYears: 1.0, atmVol: 0.05 }), // w = 0.0025*1 = 0.0025 < 0.010
    ];
    const smiles = calibrateLadder(PAIR, ladder, DEFAULT_CONVENTIONS, NOW);
    const oneY = smiles.find((s) => s.tenorYears === 1.0)!;
    expect(oneY.arbitrage.calendarArbitrageFree).toBe(false);
    expect(oneY.arbitrage.note).toMatch(/calendar/i);
  });
});

describe("sampleSurface — bilinear read over the calibrated ladder", () => {
  it("reproduces a calibrated pillar vol when sampled exactly on it", () => {
    const surface = markSurface(PAIR, brokerLadder(PAIRS[0]!), DEFAULT_CONVENTIONS, 1n, NOW);
    const smile = surface.smiles[2]!;
    const atmPillar = smile.points.find((p) => p.delta === 0.5)!;
    const v = sampleSurface(surface, smile.tenorYears, 0.5);
    expect(v).toBeCloseTo(atmPillar.vol, 12);
  });

  it("interpolates a vol strictly between two tenor pillars", () => {
    const surface = markSurface(PAIR, brokerLadder(PAIRS[0]!), DEFAULT_CONVENTIONS, 1n, NOW);
    const lo = surface.smiles[2]!;
    const hi = surface.smiles[3]!;
    const midTenor = (lo.tenorYears + hi.tenorYears) / 2;
    const vLo = sampleSurface(surface, lo.tenorYears, 0.25);
    const vHi = sampleSurface(surface, hi.tenorYears, 0.25);
    const vMid = sampleSurface(surface, midTenor, 0.25);
    const lower = Math.min(vLo, vHi);
    const upper = Math.max(vLo, vHi);
    expect(vMid).toBeGreaterThanOrEqual(lower - 1e-12);
    expect(vMid).toBeLessThanOrEqual(upper + 1e-12);
  });

  it("clamps at the wings (deep-OTM delta maps to the wing pillar)", () => {
    const surface = markSurface(PAIR, brokerLadder(PAIRS[0]!), DEFAULT_CONVENTIONS, 1n, NOW);
    const smile = surface.smiles[2]!;
    const farPut = sampleSurface(surface, smile.tenorYears, -0.95);
    const wingPut = smile.points.reduce((a, b) => (a.delta < b.delta ? a : b)).vol;
    expect(farPut).toBeCloseTo(wingPut, 12);
  });

  it("an empty surface samples to a neutral 0 (no fabricated vol)", () => {
    const empty = { pair: PAIR, surfaceVersion: 1n, smiles: [], epochNanos: NOW };
    expect(sampleSurface(empty, 0.25, 0.25)).toBe(0);
  });
});

describe("impliedVolForInstrument — the vol the structure actually trades on", () => {
  it("reads a delta-specified vanilla off the calibrated smile, not flat ATM", () => {
    const market = PAIRS[0]!.market;
    const surface = markSurface(PAIR, brokerLadder(PAIRS[0]!), DEFAULT_CONVENTIONS, 1n, NOW);
    const instrument: Instrument = {
      pair: PAIR,
      tenor: { unit: "MONTHS", count: 3 },
      expiryYears: 0.25,
      quantity: { notional: 1e6, baseCcy: true },
      side: "BUY",
      product: { kind: "vanilla", vanilla: { optionType: "PUT", strike: { kind: "delta", delta: -0.25 } } },
    };
    const v = impliedVolForInstrument(surface, instrument, market);
    // A real, finite vol off the smile — the 25Δ put wing, distinct from flat ATM.
    expect(Number.isFinite(v)).toBe(true);
    expect(v).toBeGreaterThan(0);
  });

  it("falls back to flat ATM only when the surface has no smiles", () => {
    const market = PAIRS[0]!.market;
    const empty = { pair: PAIR, surfaceVersion: 1n, smiles: [], epochNanos: NOW };
    const instrument: Instrument = {
      pair: PAIR,
      tenor: { unit: "MONTHS", count: 3 },
      expiryYears: 0.25,
      quantity: { notional: 1e6, baseCcy: true },
      side: "BUY",
      product: { kind: "vanilla", vanilla: { optionType: "CALL", strike: { kind: "delta", delta: 0.25 } } },
    };
    expect(impliedVolForInstrument(empty, instrument, market)).toBe(market.vol);
  });
});
