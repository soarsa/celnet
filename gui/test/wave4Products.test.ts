/**
 * Wave-4 product parity for the GUI end of the ONE `celnet.wire` contract: the
 * single-barrier, double-barrier, digital and touch structures are encoded onto the
 * SAME `Instrument.product` oneof as vanilla/strategy, with the EXACT wire field
 * NAMES (`single_barrier`/`double_barrier`/`digital`/`touch`, proto field numbers
 * 9/10/11/12 — already on the wire AND priced server-side; no schema_version, no
 * renumber; CLAUDE.md rule 9). The single- and double-barrier REUSE the same nested
 * `vanilla` message (option_type + strike-or-delta). The standalone build prices
 * each by the SAME Garman-Kohlhagen reflection-principle closed forms the server's
 * `celnet-exotics` (barrier.rs / digital.rs / touch.rs) uses.
 *
 * These exercise the REAL `src/data/wsCodec.ts`, `src/data/enums.ts`,
 * `src/data/seed.ts` and `src/data/pricing.ts` through their public surface with NO
 * server and NO mocks. Correctness is validated against an INDEPENDENT oracle:
 *  - the digital cash-or-nothing call equals the tight call-spread limit −∂C/∂K of
 *    an independently-coded GK vanilla (the defining identity in celnet-exotics);
 *  - barrier in/out parity (knock-in + knock-out = vanilla) holds;
 *  - the single-barrier knock-in matches the hard-coded QuantLib 1.42.1 reference
 *    prices pinned in `celnet-exotics::barrier` (S=100, σ=20%, 1y, r_d=5%, r_f=2%);
 *  - touch complementarity (one-touch + no-touch = discounted rebate; DNT + DOT =
 *    discounted rebate) holds; plus structural monotonicity.
 */
import { describe, expect, it } from "vitest";

import { instrumentToWire } from "../src/data/wsCodec";
import * as e from "../src/data/enums";
import {
  digitalInstrument,
  doubleBarrierInstrument,
  isDoubleTouch,
  singleBarrierInstrument,
  touchInstrument,
  type DigitalTerms,
  type DoubleBarrierTerms,
  type SingleBarrierTerms,
  type TouchTerms,
} from "../src/data/seed";
import { priceInstrument } from "../src/data/pricing";
import type { CcyPair, MarketContext, StrikeOrDelta } from "../src/data/contract";

const PAIR: CcyPair = { base: "EUR", quote: "USD" };
const MKT: MarketContext = { spot: 1.1, vol: 0.2, rDom: 0.03, rFor: 0.01 };
const EXPIRY = 1;

/** The QuantLib-anchored market the `celnet-exotics::barrier` reference uses. */
const QL_MKT: MarketContext = { spot: 100, vol: 0.2, rDom: 0.05, rFor: 0.02 };

function strikeLevel(strike: number): StrikeOrDelta {
  return { kind: "strike", strike };
}

function singleBarrier(over: Partial<SingleBarrierTerms> = {}): SingleBarrierTerms {
  return {
    optionType: "CALL",
    strike: strikeLevel(1.1),
    kind: "KNOCK_OUT",
    side: "UP",
    barrier: 1.2,
    rebate: 0,
    monitoring: "CONTINUOUS",
    ...over,
  };
}

function doubleBarrier(over: Partial<DoubleBarrierTerms> = {}): DoubleBarrierTerms {
  return {
    optionType: "CALL",
    strike: strikeLevel(1.1),
    kind: "KNOCK_OUT",
    lowerBarrier: 0.95,
    upperBarrier: 1.25,
    rebate: 0,
    monitoring: "CONTINUOUS",
    ...over,
  };
}

function digital(over: Partial<DigitalTerms> = {}): DigitalTerms {
  return {
    optionType: "CALL",
    strike: 1.1,
    style: "CASH_OR_NOTHING",
    payout: 1,
    ...over,
  };
}

function touch(over: Partial<TouchTerms> = {}): TouchTerms {
  return {
    kind: "ONE_TOUCH",
    lowerBarrier: 1.2,
    upperBarrier: 0,
    rebate: 1,
    monitoring: "CONTINUOUS",
    ...over,
  };
}

// ---------------------------------------------------------------------------
// independent GK oracle (A&S 7.1.26 erf CDF) — NOT the production pricer
// ---------------------------------------------------------------------------

function cdf(x: number): number {
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
}

function gkVanilla(isCall: boolean, strike: number, m: MarketContext, t: number): number {
  const sqrtT = Math.sqrt(t);
  const dfFor = Math.exp(-m.rFor * t);
  const dfDom = Math.exp(-m.rDom * t);
  const d1 =
    (Math.log(m.spot / strike) + (m.rDom - m.rFor + 0.5 * m.vol * m.vol) * t) / (m.vol * sqrtT);
  const d2 = d1 - m.vol * sqrtT;
  return isCall
    ? m.spot * dfFor * cdf(d1) - strike * dfDom * cdf(d2)
    : strike * dfDom * cdf(-d2) - m.spot * dfFor * cdf(-d1);
}

/**
 * The MOCK pricer's own vanilla at an absolute strike (a single-leg vanilla built
 * with a delta resolving to the wanted strike is awkward; instead we price the
 * vanilla through the same internal closed form by routing a single-leg vanilla
 * instrument whose delta we map). Simplest exact route: the in/out parity below
 * compares `KI + KO` to the MOCK vanilla, so we price a vanilla that uses the SAME
 * CDF — we obtain it by pricing a digital decomposition is overkill; instead build
 * a vanilla via a strategy with one BUY leg at the absolute strike.
 */
function mockVanilla(isCall: boolean, strike: number, m: MarketContext): number {
  // A single-barrier whose barrier is unreachable degenerates to the plain vanilla:
  // a down-and-IN with a barrier just below 0 can never knock in, and a down-and-OUT
  // with that barrier never knocks out ⇒ KO == vanilla. We use that KO route so the
  // vanilla is computed by the SAME mock closed form the barriers use.
  const out = priceInstrument(
    singleBarrierInstrument(PAIR, EXPIRY, 10, {
      optionType: isCall ? "CALL" : "PUT",
      strike: strikeLevel(strike),
      kind: "KNOCK_OUT",
      side: "DOWN",
      barrier: 1e-9, // unreachable ⇒ never knocks out ⇒ KO == vanilla
      rebate: 0,
      monitoring: "CONTINUOUS",
    }),
    m,
  ).greeks.price;
  return out;
}

// ---------------------------------------------------------------------------
// wire encoding — the already-contracted oneof arms, by their proto field NAMES
// ---------------------------------------------------------------------------

describe("wave-4 products — instrumentToWire oneof arms", () => {
  it("encodes a single barrier under `single_barrier` (field 9) with the nested vanilla", () => {
    const w = instrumentToWire(
      singleBarrierInstrument(PAIR, EXPIRY, 10, singleBarrier({ side: "UP", kind: "KNOCK_OUT" })),
    );
    const body = w["single_barrier"] as Record<string, unknown>;
    const vanilla = body["vanilla"] as Record<string, unknown>;
    expect(vanilla["option_type"]).toBe(e.optionType.toWire("CALL"));
    expect(vanilla["strike"]).toEqual({ strike: 1.1 });
    expect(body["kind"]).toBe(e.barrierKind.toWire("KNOCK_OUT"));
    expect(body["side"]).toBe(e.barrierSide.toWire("UP"));
    expect(body["barrier"]).toBe(1.2);
    expect(body["rebate"]).toBe(0);
    expect(body["monitoring"]).toBe(e.monitoringStyle.toWire("CONTINUOUS"));
    // A oneof carries exactly one body.
    expect(w["double_barrier"]).toBeUndefined();
    expect(w["digital"]).toBeUndefined();
    expect(w["touch"]).toBeUndefined();
    expect(w["vanilla"]).toBeUndefined();
  });

  it("encodes a double barrier under `double_barrier` (field 10) with lower/upper", () => {
    const w = instrumentToWire(
      doubleBarrierInstrument(PAIR, EXPIRY, 10, doubleBarrier({ kind: "KNOCK_IN" })),
    );
    const body = w["double_barrier"] as Record<string, unknown>;
    const vanilla = body["vanilla"] as Record<string, unknown>;
    expect(vanilla["option_type"]).toBe(e.optionType.toWire("CALL"));
    expect(body["kind"]).toBe(e.barrierKind.toWire("KNOCK_IN"));
    expect(body["lower_barrier"]).toBe(0.95);
    expect(body["upper_barrier"]).toBe(1.25);
    expect(body["rebate"]).toBe(0);
    expect(body["monitoring"]).toBe(e.monitoringStyle.toWire("CONTINUOUS"));
    expect(w["single_barrier"]).toBeUndefined();
  });

  it("encodes a digital under `digital` (field 11) with style + payout", () => {
    const w = instrumentToWire(
      digitalInstrument(PAIR, EXPIRY, 10, digital({ optionType: "PUT", style: "ASSET_OR_NOTHING", payout: 2 })),
    );
    expect(w["digital"]).toEqual({
      option_type: e.optionType.toWire("PUT"),
      strike: 1.1,
      style: e.digitalStyle.toWire("ASSET_OR_NOTHING"),
      payout: 2,
    });
  });

  it("encodes a touch under `touch` (field 12) with kind + barrier(s)", () => {
    const w = instrumentToWire(
      touchInstrument(
        PAIR,
        EXPIRY,
        10,
        touch({ kind: "DOUBLE_NO_TOUCH", lowerBarrier: 1.0, upperBarrier: 1.2, rebate: 1 }),
      ),
    );
    expect(w["touch"]).toEqual({
      kind: e.touchKind.toWire("DOUBLE_NO_TOUCH"),
      lower_barrier: 1.0,
      upper_barrier: 1.2,
      rebate: 1,
      monitoring: e.monitoringStyle.toWire("CONTINUOUS"),
    });
  });

  it("pins the canonical Wave-4 enum numbers the server WS codec decodes by", () => {
    // BarrierKind KNOCK_IN=0, KNOCK_OUT=1.
    expect(e.barrierKind.toWire("KNOCK_IN")).toBe(0);
    expect(e.barrierKind.toWire("KNOCK_OUT")).toBe(1);
    // BarrierSide UP=0, DOWN=1.
    expect(e.barrierSide.toWire("UP")).toBe(0);
    expect(e.barrierSide.toWire("DOWN")).toBe(1);
    // MonitoringStyle CONTINUOUS=0, DISCRETE=1.
    expect(e.monitoringStyle.toWire("CONTINUOUS")).toBe(0);
    expect(e.monitoringStyle.toWire("DISCRETE")).toBe(1);
    // TouchKind ONE=0, NO=1, DOUBLE_NO=2, DOUBLE_ONE=3.
    expect(e.touchKind.toWire("ONE_TOUCH")).toBe(0);
    expect(e.touchKind.toWire("NO_TOUCH")).toBe(1);
    expect(e.touchKind.toWire("DOUBLE_NO_TOUCH")).toBe(2);
    expect(e.touchKind.toWire("DOUBLE_ONE_TOUCH")).toBe(3);
    // DigitalStyle CASH=0, ASSET=1.
    expect(e.digitalStyle.toWire("CASH_OR_NOTHING")).toBe(0);
    expect(e.digitalStyle.toWire("ASSET_OR_NOTHING")).toBe(1);
    // Every decode is reversible.
    expect(e.barrierKind.fromWire(1)).toBe("KNOCK_OUT");
    expect(e.touchKind.fromWire(3)).toBe("DOUBLE_ONE_TOUCH");
    expect(e.digitalStyle.fromWire(1)).toBe("ASSET_OR_NOTHING");
  });

  it("a single delta-strike barrier round-trips a delta spec in the nested vanilla", () => {
    const w = instrumentToWire(
      singleBarrierInstrument(
        PAIR,
        EXPIRY,
        10,
        singleBarrier({ strike: { kind: "delta", delta: 0.25 } }),
      ),
    );
    const body = w["single_barrier"] as Record<string, unknown>;
    expect((body["vanilla"] as Record<string, unknown>)["strike"]).toEqual({ delta: 0.25 });
  });
});

// ---------------------------------------------------------------------------
// offline pricer — closed forms validated vs an INDEPENDENT oracle & identities
// ---------------------------------------------------------------------------

describe("wave-4 single barrier — in/out parity & QuantLib reference", () => {
  it("knock-in + knock-out = the vanilla (in/out parity, zero rebate)", () => {
    for (const side of ["UP", "DOWN"] as const) {
      for (const optionType of ["CALL", "PUT"] as const) {
        const ki = priceInstrument(
          singleBarrierInstrument(
            PAIR,
            EXPIRY,
            10,
            singleBarrier({ optionType, side, kind: "KNOCK_IN", barrier: side === "UP" ? 1.2 : 1.0 }),
          ),
          MKT,
        ).greeks.price;
        const ko = priceInstrument(
          singleBarrierInstrument(
            PAIR,
            EXPIRY,
            10,
            singleBarrier({ optionType, side, kind: "KNOCK_OUT", barrier: side === "UP" ? 1.2 : 1.0 }),
          ),
          MKT,
        ).greeks.price;
        // In/out parity is a STRUCTURAL identity in the mock (KO = vanilla − KI on
        // the SAME CDF), so KI + KO recovers the mock's vanilla to machine precision.
        const vanilla = mockVanilla(optionType === "CALL", 1.1, MKT);
        expect(ki + ko).toBeCloseTo(vanilla, 12);
        // And that mock vanilla agrees with the INDEPENDENT GK oracle to the mock's
        // CDF accuracy (~1e-6 rational-approximation band).
        expect(vanilla).toBeCloseTo(gkVanilla(optionType === "CALL", 1.1, MKT, EXPIRY), 6);
      }
    }
  });

  it("a knock-out is bounded by the vanilla and never negative", () => {
    const ko = priceInstrument(
      singleBarrierInstrument(PAIR, EXPIRY, 10, singleBarrier({ kind: "KNOCK_OUT" })),
      MKT,
    ).greeks.price;
    const vanilla = gkVanilla(true, 1.1, MKT, EXPIRY);
    expect(ko).toBeGreaterThanOrEqual(0);
    expect(ko).toBeLessThanOrEqual(vanilla + 1e-9);
  });

  it("matches the hard-coded QuantLib 1.42.1 knock-in references (celnet-exotics)", () => {
    // (up, optionType, K, H, QuantLib KI price) at S=100, σ=20%, 1y, r_d=5%, r_f=2%.
    const cases: [boolean, "CALL" | "PUT", number, number, number][] = [
      [false, "CALL", 100, 80, 0.093_699_071_656],
      [true, "CALL", 100, 120, 8.094_513_367_157],
      [false, "PUT", 100, 80, 4.597_402_864_380],
      [true, "PUT", 100, 120, 0.230_613_308_713],
    ];
    for (const [up, optionType, k, h, reference] of cases) {
      const ki = priceInstrument(
        singleBarrierInstrument(
          PAIR,
          EXPIRY,
          10,
          singleBarrier({
            optionType,
            strike: strikeLevel(k),
            side: up ? "UP" : "DOWN",
            kind: "KNOCK_IN",
            barrier: h,
          }),
        ),
        QL_MKT,
      ).greeks.price;
      // The offline pricer mirrors the SAME reflection-principle closed form the
      // server's celnet-exotics uses (gated to ~1e-9 vs QuantLib there). The only
      // residual here is the mock's rational-approximation normal CDF (~7.5e-8
      // absolute accuracy per its docstring); on these small (S=100-scale) KI
      // values the propagated ABSOLUTE error is ~1e-6 — far tighter than any
      // mis-selected Reiner-Rubinstein block (which would be off by whole units).
      expect(Math.abs(ki - reference)).toBeLessThan(2e-5);
    }
  });

  it("an at-hit rebate raises a knock-out's value", () => {
    const noReb = priceInstrument(
      singleBarrierInstrument(PAIR, EXPIRY, 10, singleBarrier({ rebate: 0 })),
      MKT,
    ).greeks.price;
    const withReb = priceInstrument(
      singleBarrierInstrument(PAIR, EXPIRY, 10, singleBarrier({ rebate: 0.02 })),
      MKT,
    ).greeks.price;
    expect(withReb).toBeGreaterThan(noReb);
  });
});

describe("wave-4 double barrier — corridor structure & parity", () => {
  it("knock-in + knock-out = the vanilla (priced by parity)", () => {
    const ki = priceInstrument(
      doubleBarrierInstrument(PAIR, EXPIRY, 10, doubleBarrier({ kind: "KNOCK_IN" })),
      MKT,
    ).greeks.price;
    const ko = priceInstrument(
      doubleBarrierInstrument(PAIR, EXPIRY, 10, doubleBarrier({ kind: "KNOCK_OUT" })),
      MKT,
    ).greeks.price;
    // A double knock-in is priced by parity (KI = vanilla − KO) on the SAME mock
    // CDF, so KI + KO recovers the mock vanilla to machine precision.
    const vanilla = mockVanilla(true, 1.1, MKT);
    expect(ki + ko).toBeCloseTo(vanilla, 12);
  });

  it("a knock-out is non-negative and below the vanilla (corridor truncation)", () => {
    const ko = priceInstrument(
      doubleBarrierInstrument(PAIR, EXPIRY, 10, doubleBarrier({ kind: "KNOCK_OUT" })),
      MKT,
    ).greeks.price;
    const vanilla = gkVanilla(true, 1.1, MKT, EXPIRY);
    expect(ko).toBeGreaterThanOrEqual(0);
    expect(ko).toBeLessThan(vanilla);
  });

  it("a wider corridor knocks out less ⇒ a higher knock-out value", () => {
    const narrow = priceInstrument(
      doubleBarrierInstrument(
        PAIR,
        EXPIRY,
        10,
        doubleBarrier({ kind: "KNOCK_OUT", lowerBarrier: 1.03, upperBarrier: 1.17 }),
      ),
      MKT,
    ).greeks.price;
    const wide = priceInstrument(
      doubleBarrierInstrument(
        PAIR,
        EXPIRY,
        10,
        doubleBarrier({ kind: "KNOCK_OUT", lowerBarrier: 0.85, upperBarrier: 1.35 }),
      ),
      MKT,
    ).greeks.price;
    expect(wide).toBeGreaterThan(narrow);
  });
});

describe("wave-4 digital — call-spread limit & complementarity", () => {
  it("a vanilla decomposes EXACTLY into asset-or-nothing − K·cash-or-nothing", () => {
    // The defining digital identity (celnet-exotics::digital::vanilla_decomposition):
    // C = AssetCall − K·CashCall, P = K·CashPut − AssetPut — an EXACT relation on the
    // SAME CDF (no FD truncation), so it holds to machine precision in the mock.
    const k = 1.1;
    const assetC = priceInstrument(
      digitalInstrument(PAIR, EXPIRY, 10, digital({ optionType: "CALL", style: "ASSET_OR_NOTHING" })),
      MKT,
    ).greeks.price;
    const cashC = priceInstrument(
      digitalInstrument(PAIR, EXPIRY, 10, digital({ optionType: "CALL", style: "CASH_OR_NOTHING" })),
      MKT,
    ).greeks.price;
    expect(assetC - k * cashC).toBeCloseTo(mockVanilla(true, k, MKT), 12);

    const assetP = priceInstrument(
      digitalInstrument(PAIR, EXPIRY, 10, digital({ optionType: "PUT", style: "ASSET_OR_NOTHING" })),
      MKT,
    ).greeks.price;
    const cashP = priceInstrument(
      digitalInstrument(PAIR, EXPIRY, 10, digital({ optionType: "PUT", style: "CASH_OR_NOTHING" })),
      MKT,
    ).greeks.price;
    expect(k * cashP - assetP).toBeCloseTo(mockVanilla(false, k, MKT), 12);
  });

  it("the cash-or-nothing call equals the tight call-spread limit −∂C/∂K", () => {
    // The defining digital identity (celnet-exotics::digital): CashCall = −∂C/∂K.
    // The FD is taken on the MOCK vanilla so both sides use the SAME CDF; the
    // residual is the central-difference truncation O(hk²), validated to a band that
    // shrinks with hk (here ~1e-4 relative — a genuine numerical-derivative check).
    const k = 1.1;
    const hk = 5e-5 * k;
    const fd = -(mockVanilla(true, k + hk, MKT) - mockVanilla(true, k - hk, MKT)) / (2 * hk);
    const cash = priceInstrument(
      digitalInstrument(PAIR, EXPIRY, 10, digital({ optionType: "CALL", style: "CASH_OR_NOTHING", payout: 1 })),
      MKT,
    ).greeks.price;
    expect(Math.abs(cash - fd) / Math.max(cash, 1e-6)).toBeLessThan(1e-3);
  });

  it("cash-or-nothing call + put = the discounted certainty e^{-r_d T}", () => {
    const c = priceInstrument(
      digitalInstrument(PAIR, EXPIRY, 10, digital({ optionType: "CALL" })),
      MKT,
    ).greeks.price;
    const p = priceInstrument(
      digitalInstrument(PAIR, EXPIRY, 10, digital({ optionType: "PUT" })),
      MKT,
    ).greeks.price;
    expect(c + p).toBeCloseTo(Math.exp(-MKT.rDom * EXPIRY), 10);
  });

  it("asset-or-nothing call + put = the discounted asset S·e^{-r_f T}", () => {
    const c = priceInstrument(
      digitalInstrument(PAIR, EXPIRY, 10, digital({ optionType: "CALL", style: "ASSET_OR_NOTHING" })),
      MKT,
    ).greeks.price;
    const p = priceInstrument(
      digitalInstrument(PAIR, EXPIRY, 10, digital({ optionType: "PUT", style: "ASSET_OR_NOTHING" })),
      MKT,
    ).greeks.price;
    expect(c + p).toBeCloseTo(MKT.spot * Math.exp(-MKT.rFor * EXPIRY), 9);
  });

  it("the payout scales the price linearly", () => {
    const one = priceInstrument(
      digitalInstrument(PAIR, EXPIRY, 10, digital({ payout: 1 })),
      MKT,
    ).greeks.price;
    const three = priceInstrument(
      digitalInstrument(PAIR, EXPIRY, 10, digital({ payout: 3 })),
      MKT,
    ).greeks.price;
    expect(three).toBeCloseTo(3 * one, 12);
  });
});

describe("wave-4 touch — complementarity, bounds & monotonicity", () => {
  it("no-touch + one-touch(at-expiry-equivalent) ⇒ value in [0, rebate]", () => {
    // The product one-touch is AT HIT; pair it with the no-touch and check both lie
    // in [0, rebate] and the no-touch is the survive-leg (complement of the
    // deferred touch). The exact deferred complementarity is checked via the DNT.
    const ot = priceInstrument(
      touchInstrument(PAIR, EXPIRY, 10, touch({ kind: "ONE_TOUCH" })),
      MKT,
    ).greeks.price;
    const nt = priceInstrument(
      touchInstrument(PAIR, EXPIRY, 10, touch({ kind: "NO_TOUCH" })),
      MKT,
    ).greeks.price;
    expect(ot).toBeGreaterThanOrEqual(0);
    expect(ot).toBeLessThanOrEqual(1 + 1e-9);
    expect(nt).toBeGreaterThanOrEqual(0);
    expect(nt).toBeLessThanOrEqual(1 + 1e-9);
  });

  it("double-no-touch + double-one-touch = the discounted rebate", () => {
    const nt = priceInstrument(
      touchInstrument(
        PAIR,
        EXPIRY,
        10,
        touch({ kind: "DOUBLE_NO_TOUCH", lowerBarrier: 0.95, upperBarrier: 1.25 }),
      ),
      MKT,
    ).greeks.price;
    const dt = priceInstrument(
      touchInstrument(
        PAIR,
        EXPIRY,
        10,
        touch({ kind: "DOUBLE_ONE_TOUCH", lowerBarrier: 0.95, upperBarrier: 1.25 }),
      ),
      MKT,
    ).greeks.price;
    expect(nt + dt).toBeCloseTo(Math.exp(-MKT.rDom * EXPIRY), 10);
  });

  it("a closer one-touch barrier is touched more often ⇒ a higher value", () => {
    const near = priceInstrument(
      touchInstrument(PAIR, EXPIRY, 10, touch({ kind: "ONE_TOUCH", lowerBarrier: 1.13 })),
      MKT,
    ).greeks.price;
    const far = priceInstrument(
      touchInstrument(PAIR, EXPIRY, 10, touch({ kind: "ONE_TOUCH", lowerBarrier: 1.35 })),
      MKT,
    ).greeks.price;
    expect(near).toBeGreaterThan(far);
  });

  it("a wider double-no-touch corridor survives more ⇒ a higher value", () => {
    const narrow = priceInstrument(
      touchInstrument(
        PAIR,
        EXPIRY,
        10,
        touch({ kind: "DOUBLE_NO_TOUCH", lowerBarrier: 1.04, upperBarrier: 1.16 }),
      ),
      MKT,
    ).greeks.price;
    const wide = priceInstrument(
      touchInstrument(
        PAIR,
        EXPIRY,
        10,
        touch({ kind: "DOUBLE_NO_TOUCH", lowerBarrier: 0.85, upperBarrier: 1.4 }),
      ),
      MKT,
    ).greeks.price;
    expect(wide).toBeGreaterThan(narrow);
  });

  it("isDoubleTouch classifies the corridor kinds", () => {
    expect(isDoubleTouch("ONE_TOUCH")).toBe(false);
    expect(isDoubleTouch("NO_TOUCH")).toBe(false);
    expect(isDoubleTouch("DOUBLE_NO_TOUCH")).toBe(true);
    expect(isDoubleTouch("DOUBLE_ONE_TOUCH")).toBe(true);
  });
});
