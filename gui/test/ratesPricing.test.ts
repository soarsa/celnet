import { describe, expect, it } from "vitest";

import type { OisInstrument, RatesCurveSet } from "../src/data/contract";
import { yearsPillarTenor } from "../src/data/contract";
import {
  DEFAULT_USD_SOFR_CURVE,
  priceRatesOffline,
  RatesPricingError,
} from "../src/data/ratesPricing";

/**
 * Structural-identity tests for the in-browser OIS pricer. We assert the
 * arbitrage-free identities a real discounting engine must satisfy — a swap
 * struck at its own par rate has zero PV, payer/receiver are exact mirrors,
 * the key-rate ladder reconciles to the parallel DV01, risk scales linearly in
 * notional, and a richer/leaner fixed rate moves the receiver monotonically —
 * rather than pinning opaque magic numbers. This is the same validation
 * discipline the Rust `celnet-rates` engine the live edge uses is held to.
 */

const NOTIONAL = 10_000_000;

function ois(overrides: Partial<OisInstrument> = {}): OisInstrument {
  return {
    tenorYears: 5,
    fixedRate: 0.04,
    notional: NOTIONAL,
    direction: "RECEIVE_FIXED",
    ...overrides,
  };
}

describe("priceRatesOffline — arbitrage-free identities", () => {
  it("prices a swap struck at its own par rate to ~zero PV", () => {
    // Discover the fair fixed rate, then re-strike at it: PV must vanish.
    const par = priceRatesOffline(DEFAULT_USD_SOFR_CURVE, ois()).parRate;
    const atPar = priceRatesOffline(
      DEFAULT_USD_SOFR_CURVE,
      ois({ fixedRate: par }),
    );
    // ~1e-9 relative on a 10mm notional.
    expect(Math.abs(atPar.pv)).toBeLessThan(1e-2);
  });

  it("reports a par rate independent of pay/receive direction", () => {
    const recv = priceRatesOffline(
      DEFAULT_USD_SOFR_CURVE,
      ois({ direction: "RECEIVE_FIXED" }),
    );
    const pay = priceRatesOffline(
      DEFAULT_USD_SOFR_CURVE,
      ois({ direction: "PAY_FIXED" }),
    );
    expect(pay.parRate).toBeCloseTo(recv.parRate, 12);
  });

  it("makes payer and receiver exact mirrors across every measure", () => {
    const recv = priceRatesOffline(
      DEFAULT_USD_SOFR_CURVE,
      ois({ direction: "RECEIVE_FIXED" }),
    );
    const pay = priceRatesOffline(
      DEFAULT_USD_SOFR_CURVE,
      ois({ direction: "PAY_FIXED" }),
    );
    expect(pay.pv).toBeCloseTo(-recv.pv, 6);
    expect(pay.pv01).toBeCloseTo(-recv.pv01, 6);
    expect(pay.dv01).toBeCloseTo(-recv.dv01, 6);
    expect(pay.keyRateLadder.length).toBe(recv.keyRateLadder.length);
    pay.keyRateLadder.forEach((k, i) => expect(k).toBeCloseTo(-recv.keyRateLadder[i]!, 6));
  });

  it("reconciles the key-rate ladder to the parallel DV01", () => {
    // The sum of independent per-pillar 1bp bumps equals the single parallel 1bp
    // bump to first order; they differ only at second order (the swap is mildly
    // convex in the zero rates), so reconcile on a tight relative tolerance.
    const r = priceRatesOffline(DEFAULT_USD_SOFR_CURVE, ois());
    const ladderSum = r.keyRateLadder.reduce((a, b) => a + b, 0);
    expect(Math.abs(ladderSum - r.dv01) / Math.abs(r.dv01)).toBeLessThan(1e-3);
  });

  it("scales PV / PV01 / DV01 linearly in notional", () => {
    const base = priceRatesOffline(
      DEFAULT_USD_SOFR_CURVE,
      ois({ fixedRate: 0.05 }),
    );
    const dbl = priceRatesOffline(
      DEFAULT_USD_SOFR_CURVE,
      ois({ fixedRate: 0.05, notional: 2 * NOTIONAL }),
    );
    expect(dbl.pv).toBeCloseTo(2 * base.pv, 6);
    expect(dbl.pv01).toBeCloseTo(2 * base.pv01, 6);
    expect(dbl.dv01).toBeCloseTo(2 * base.dv01, 6);
  });

  it("moves the receiver PV monotonically up with the fixed rate received", () => {
    // Receiving a richer fixed coupon is worth strictly more.
    const lo = priceRatesOffline(
      DEFAULT_USD_SOFR_CURVE,
      ois({ fixedRate: 0.03 }),
    );
    const mid = priceRatesOffline(
      DEFAULT_USD_SOFR_CURVE,
      ois({ fixedRate: 0.04 }),
    );
    const hi = priceRatesOffline(
      DEFAULT_USD_SOFR_CURVE,
      ois({ fixedRate: 0.05 }),
    );
    expect(lo.pv).toBeLessThan(mid.pv);
    expect(mid.pv).toBeLessThan(hi.pv);
  });

  it("reproduces the calibrating pillar's par rate at a pillar tenor", () => {
    // The 5Y pillar quote is 4.05%; the bootstrapped 5Y swap par must match it
    // to a few bp (exact reproduction up to the schedule's day-count rolling).
    const r = priceRatesOffline(DEFAULT_USD_SOFR_CURVE, ois({ tenorYears: 5 }));
    expect(r.parRate).toBeGreaterThan(0.039);
    expect(r.parRate).toBeLessThan(0.042);
  });
});

describe("priceRatesOffline — input validation (rejects exactly as the server does)", () => {
  const bad = (curve: RatesCurveSet, inst: OisInstrument) => () =>
    priceRatesOffline(curve, inst);

  it("rejects an unsupported currency", () => {
    const curve: RatesCurveSet = { ...DEFAULT_USD_SOFR_CURVE, currency: "EUR" };
    expect(bad(curve, ois())).toThrow(RatesPricingError);
  });

  it("rejects an empty pillar set", () => {
    const curve: RatesCurveSet = { ...DEFAULT_USD_SOFR_CURVE, pillars: [] };
    expect(bad(curve, ois())).toThrow(RatesPricingError);
  });

  it("rejects non-strictly-increasing pillar tenors", () => {
    const curve: RatesCurveSet = {
      ...DEFAULT_USD_SOFR_CURVE,
      pillars: [
        { tenor: yearsPillarTenor(2), parRate: 0.04 },
        { tenor: yearsPillarTenor(2), parRate: 0.041 },
      ],
    };
    expect(bad(curve, ois())).toThrow(RatesPricingError);
  });

  it("rejects a sub-1Y instrument tenor and a non-positive notional", () => {
    expect(bad(DEFAULT_USD_SOFR_CURVE, ois({ tenorYears: 0 }))).toThrow(
      RatesPricingError,
    );
    expect(bad(DEFAULT_USD_SOFR_CURVE, ois({ notional: 0 }))).toThrow(
      RatesPricingError,
    );
  });
});
