import { describe, expect, it } from "vitest";

import type {
  BondInstrument,
  FraInstrument,
  OisInstrument,
  RatesCurveSet,
  VanillaIrsInstrument,
} from "../src/data/contract";
import { yearsPillarTenor } from "../src/data/contract";
import {
  DEFAULT_USD_SOFR_CURVE,
  priceBondOffline,
  priceFraOffline,
  priceIrsOffline,
  priceRatesInstrumentOffline,
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

// ===========================================================================
// the additive arms — vanilla IRS, FRA, cash bond (fi-bond-ticket-gui)
// ===========================================================================
//
// Structural-identity tests: each offline arm reproduces the server engine it
// wraps, so we assert the arbitrage-free identities the real engine satisfies
// (a par instrument prices to zero, payer/receiver mirror, the key-rate ladder
// reconciles to the parallel DV01, an annual swap collapses to the OIS) rather
// than pinning opaque magic numbers.

const CURVE = DEFAULT_USD_SOFR_CURVE;

function irs(overrides: Partial<VanillaIrsInstrument> = {}): VanillaIrsInstrument {
  return {
    tenorYears: 5,
    fixedRate: 0.04,
    notional: 10_000_000,
    direction: "RECEIVE_FIXED",
    fixedFrequency: "SEMI_ANNUAL",
    fixedDayCount: "ACT_360",
    floatFrequency: "QUARTERLY",
    floatDayCount: "ACT_360",
    ...overrides,
  };
}

describe("priceIrsOffline — arbitrage-free identities", () => {
  it("prices a swap struck at its own par rate to ~zero PV", () => {
    const par = priceIrsOffline(CURVE, irs()).parRate;
    const atPar = priceIrsOffline(CURVE, irs({ fixedRate: par }));
    expect(Math.abs(atPar.pv)).toBeLessThan(1e-2);
  });

  it("makes payer and receiver exact mirrors across every measure", () => {
    const recv = priceIrsOffline(CURVE, irs({ direction: "RECEIVE_FIXED", fixedRate: 0.045 }));
    const pay = priceIrsOffline(CURVE, irs({ direction: "PAY_FIXED", fixedRate: 0.045 }));
    expect(pay.pv).toBeCloseTo(-recv.pv, 6);
    expect(pay.pv01).toBeCloseTo(-recv.pv01, 6);
    expect(pay.dv01).toBeCloseTo(-recv.dv01, 6);
    expect(pay.parRate).toBeCloseTo(recv.parRate, 12);
  });

  it("collapses to the OIS when it is an annual ACT/360 fixed-vs-annual-ACT/360 float", () => {
    // An annual ACT/360 fixed leg vs an annual ACT/360 float leg IS the self-
    // discounting OIS, so its par rate + PV must equal the OIS arm's to ~1e-12.
    const annual = priceIrsOffline(
      CURVE,
      irs({ fixedFrequency: "ANNUAL", floatFrequency: "ANNUAL" }),
    );
    const ois = priceRatesOffline(CURVE, {
      tenorYears: 5,
      fixedRate: 0.04,
      notional: 10_000_000,
      direction: "RECEIVE_FIXED",
    });
    expect(annual.parRate).toBeCloseTo(ois.parRate, 12);
    expect(annual.pv).toBeCloseTo(ois.pv, 6);
  });

  it("reconciles the key-rate ladder to the parallel DV01", () => {
    const res = priceIrsOffline(CURVE, irs({ fixedRate: 0.041 }));
    expect(res.keyRateLadder.length).toBe(CURVE.pillars.length);
    const summed = res.keyRateLadder.reduce((a, b) => a + b, 0);
    expect(Math.abs(summed - res.dv01)).toBeLessThan(1e-6 * Math.abs(res.dv01));
  });

  it("rejects a sub-1Y tenor and a non-positive notional", () => {
    expect(() => priceIrsOffline(CURVE, irs({ tenorYears: 0 }))).toThrow(RatesPricingError);
    expect(() => priceIrsOffline(CURVE, irs({ notional: 0 }))).toThrow(RatesPricingError);
  });
});

function fra(overrides: Partial<FraInstrument> = {}): FraInstrument {
  return {
    startMonths: 3,
    endMonths: 6,
    fixedRate: 0.043,
    notional: 10_000_000,
    direction: "RECEIVE_FIXED",
    accrualBasis: "ACT_360",
    ...overrides,
  };
}

describe("priceFraOffline — arbitrage-free identities", () => {
  it("prices a FRA struck at its own par (break-even) rate to ~zero PV", () => {
    const par = priceFraOffline(CURVE, fra()).parRate;
    const atPar = priceFraOffline(CURVE, fra({ fixedRate: par }));
    expect(Math.abs(atPar.pv)).toBeLessThan(1e-4);
  });

  it("makes payer and receiver exact mirrors", () => {
    const recv = priceFraOffline(CURVE, fra({ direction: "RECEIVE_FIXED" }));
    const pay = priceFraOffline(CURVE, fra({ direction: "PAY_FIXED" }));
    expect(pay.pv).toBeCloseTo(-recv.pv, 6);
    expect(pay.dv01).toBeCloseTo(-recv.dv01, 6);
    expect(pay.parRate).toBeCloseTo(recv.parRate, 12);
  });

  it("prices the 30/360 accrual to a higher par than ACT/360 (smaller tau ⇒ higher par)", () => {
    const act = priceFraOffline(CURVE, fra({ accrualBasis: "ACT_360" })).parRate;
    const thirty = priceFraOffline(CURVE, fra({ accrualBasis: "THIRTY_360_BOND_BASIS" })).parRate;
    expect(thirty).toBeGreaterThan(act);
  });

  it("reconciles the key-rate ladder to the parallel DV01", () => {
    const res = priceFraOffline(CURVE, fra());
    expect(res.keyRateLadder.length).toBe(CURVE.pillars.length);
    const summed = res.keyRateLadder.reduce((a, b) => a + b, 0);
    expect(Math.abs(summed - res.dv01)).toBeLessThan(1e-6 * Math.abs(res.dv01));
  });

  it("rejects a non-increasing window and a non-positive notional", () => {
    expect(() => priceFraOffline(CURVE, fra({ startMonths: 6, endMonths: 6 }))).toThrow(
      RatesPricingError,
    );
    expect(() => priceFraOffline(CURVE, fra({ notional: 0 }))).toThrow(RatesPricingError);
  });
});

function bond(overrides: Partial<BondInstrument> = {}): BondInstrument {
  return {
    couponRate: 0.04,
    couponFrequency: "SEMI_ANNUAL",
    dayCount: "THIRTY_360_BOND_BASIS",
    maturityDate: { year: 2031, month: 6, day: 25 },
    redemption: 100,
    position: "LONG",
    ...overrides,
  };
}

describe("priceBondOffline — curve-discounted dirty price + yield risk", () => {
  it("maps the wire result: pv = dirty price, parRate = yield, empty ladder", () => {
    const res = priceBondOffline(CURVE, bond());
    expect(res.pv).toBeGreaterThan(0);
    expect(res.parRate).toBeGreaterThan(0);
    expect(res.pv01).toBe(res.dv01); // the bond PV01 ≡ its yield DV01
    expect(res.keyRateLadder).toEqual([]);
  });

  it("makes a long and a short exact mirrors (yield is side-independent)", () => {
    const long = priceBondOffline(CURVE, bond({ position: "LONG" }));
    const short = priceBondOffline(CURVE, bond({ position: "SHORT" }));
    expect(short.pv).toBeCloseTo(-long.pv, 9);
    expect(short.dv01).toBeCloseTo(-long.dv01, 9);
    expect(short.parRate).toBeCloseTo(long.parRate, 12);
  });

  it("prices a richer coupon higher and a zero-coupon below redemption", () => {
    const base = priceBondOffline(CURVE, bond({ couponRate: 0.04 }));
    const rich = priceBondOffline(CURVE, bond({ couponRate: 0.08 }));
    expect(rich.pv).toBeGreaterThan(base.pv);
    const zero = priceBondOffline(CURVE, bond({ couponRate: 0 }));
    expect(zero.pv).toBeLessThan(100); // a positive-rate zero prices at a discount
    expect(zero.parRate).toBeGreaterThan(0);
  });

  it("prices a par-ish 5y near redemption (coupon ≈ curve level)", () => {
    // The default USD-SOFR 5y par level is ~4.05%; a 4% semi bond maturing ~5y out
    // discounts close to par off that curve — a real-magnitude sanity band.
    const res = priceBondOffline(CURVE, bond({ couponRate: 0.04 }));
    expect(res.pv).toBeGreaterThan(90);
    expect(res.pv).toBeLessThan(105);
  });

  it("rejects a maturity on/before settlement and a non-positive redemption", () => {
    expect(() =>
      priceBondOffline(CURVE, bond({ maturityDate: { year: 2020, month: 1, day: 1 } })),
    ).toThrow(RatesPricingError);
    expect(() => priceBondOffline(CURVE, bond({ redemption: 0 }))).toThrow(RatesPricingError);
  });
});

describe("priceRatesInstrumentOffline — the oneof dispatcher", () => {
  it("routes each arm to its engine, matching the direct call", () => {
    const o: OisInstrument = {
      tenorYears: 5,
      fixedRate: 0.04,
      notional: 10_000_000,
      direction: "RECEIVE_FIXED",
    };
    expect(priceRatesInstrumentOffline(CURVE, { kind: "ois", ois: o })).toEqual(
      priceRatesOffline(CURVE, o),
    );
    expect(priceRatesInstrumentOffline(CURVE, { kind: "irs", irs: irs() })).toEqual(
      priceIrsOffline(CURVE, irs()),
    );
    expect(priceRatesInstrumentOffline(CURVE, { kind: "fra", fra: fra() })).toEqual(
      priceFraOffline(CURVE, fra()),
    );
    expect(priceRatesInstrumentOffline(CURVE, { kind: "bond", bond: bond() })).toEqual(
      priceBondOffline(CURVE, bond()),
    );
  });
});
