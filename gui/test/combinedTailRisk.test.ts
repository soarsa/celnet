import { describe, expect, it } from "vitest";

import type {
  CombinedTailRiskRequest,
  CombinedTailRiskResponse,
  JointTailScenario,
  TailRiskCurvePillar,
  TailRiskFiPosition,
  TailRiskOptionLeg,
} from "../src/data/contract";
import {
  combinedTailRiskRequestToWire,
  combinedTailRiskRequestFromWire,
  combinedTailRiskResponseToWire,
  combinedTailRiskResponseFromWire,
} from "../src/data/wsCodec";
import { createMockTransport } from "../src/data/mockSource";
import {
  combinedTailRiskOffline,
  defaultTailRiskBaseCurve,
  RatesPricingError,
} from "../src/data/ratesPricing";
import {
  buildJointScenarios,
  seedTailRiskFiPositions,
  seedTailRiskOptionLegs,
} from "../src/workspaces/RiskWorkspace";
import { DEFAULT_CONVENTIONS } from "../src/data/seed";

/**
 * Two-part discipline for `RiskService.CombinedTailRisk` (the C2c joint options+FI
 * tail cube): (1) the browser JSON codec is the exact byte-mirror of the server's
 * descriptor-driven `ws::generated_codec` — snake_case keys, numeric enum tags,
 * the FI oneof keyed `ois_swap`, and a clean round-trip through both directions;
 * (2) the offline engine reproduces `celnet_risk_cube::fi::combined_tail_risk` and
 * satisfies its reduction identities — options-only ⇒ the options VaR (no FI
 * axis), FI-only ⇒ the rate VaR (a reconciling ladder), the ladder + parallel
 * DV01 depend only on the FI book, and every measure is positively homogeneous in
 * the notionals. Structural identities, not pinned magic numbers.
 */

// --- fixtures ---------------------------------------------------------------

const EURUSD = { base: "EUR", quote: "USD" };
const MARKET = { spot: 1.0768, vol: 0.0755, rDom: 0.0432, rFor: 0.0218 };

function baseCurve(): TailRiskCurvePillar[] {
  return [
    { t: 1, zeroRate: 0.043 },
    { t: 2, zeroRate: 0.041 },
    { t: 5, zeroRate: 0.04 },
    { t: 10, zeroRate: 0.042 },
  ];
}

/** A receive-fixed OIS swap to `tenor` years, annual fixed leg from the origin. */
function swap(
  tenor: number,
  fixedRate: number,
  notional: number,
  receiveFixed: boolean,
): TailRiskFiPosition {
  return {
    oisSwap: {
      start: 0,
      periods: Array.from({ length: tenor }, (_, i) => ({
        pay: i + 1,
        accrual: 1,
      })),
      fixedRate,
      notional,
      receiveFixed,
    },
  };
}

function optionLeg(over: Partial<TailRiskOptionLeg> = {}): TailRiskOptionLeg {
  return {
    pair: EURUSD,
    optionType: "CALL",
    notionalBase: 10_000_000,
    spot: 1.08,
    strike: 1.08,
    vol: 0.09,
    t: 0.25,
    rDom: 0.043,
    rFor: 0.022,
    quotedDelta: "SPOT_UNADJUSTED",
    premiumStyle: "DOMESTIC_PIPS",
    ...over,
  };
}

/** A short aligned joint-scenario set over `n` pillars (spot/vol/rate co-moves). */
function scenarios(n: number): JointTailScenario[] {
  const specs = [
    { spotRel: -0.03, volAbs: 0.03, rate: -0.002 },
    { spotRel: 0.02, volAbs: -0.01, rate: 0.0015 },
    { spotRel: -0.01, volAbs: 0.01, rate: 0.004 },
    { spotRel: 0.01, volAbs: -0.005, rate: -0.004 },
    { spotRel: 0, volAbs: 0, rate: 0 },
  ];
  return specs.map((s) => ({
    spotRel: s.spotRel,
    volAbs: s.volAbs,
    discountAbs: s.rate,
    carryAbs: 0.0005,
    rateShifts: Array.from({ length: n }, () => s.rate),
  }));
}

function request(over: Partial<CombinedTailRiskRequest> = {}): CombinedTailRiskRequest {
  const curve = baseCurve();
  return {
    optionLegs: [optionLeg(), optionLeg({ optionType: "PUT", notionalBase: -8_000_000, strike: 1.05 })],
    fiPositions: [swap(5, 0.041, 100_000_000, true), swap(2, 0.045, 50_000_000, false)],
    baseCurve: curve,
    scenarios: scenarios(curve.length),
    alpha: 0.99,
    ...over,
  };
}

// --- (1) codec: byte-shape + round-trip -------------------------------------

describe("CombinedTailRisk codec — the WS byte-mirror", () => {
  it("encodes the request in snake_case with numeric enums + the ois_swap oneof", () => {
    const wire = combinedTailRiskRequestToWire(request()) as Record<string, unknown>;
    expect(Object.keys(wire).sort()).toEqual(
      ["alpha", "base_curve", "fi_positions", "option_legs", "scenarios"].sort(),
    );

    const leg = (wire.option_legs as Record<string, unknown>[])[0]!;
    expect(leg.pair).toEqual({ base: "EUR", quote: "USD" });
    expect(leg.option_type).toBe(0); // CALL = 0
    expect(leg.notional_base).toBe(10_000_000);
    expect(leg.r_dom).toBe(0.043);
    expect(leg.r_for).toBe(0.022);
    expect(leg.quoted_delta).toBe(0); // SPOT_UNADJUSTED = 0
    expect(leg.premium_style).toBe(0); // DOMESTIC_PIPS = 0

    // The FI `position` oneof carries ONLY the live arm's key, `ois_swap`.
    const fi = (wire.fi_positions as Record<string, unknown>[])[0]!;
    expect(Object.keys(fi)).toEqual(["ois_swap"]);
    const ois = fi.ois_swap as Record<string, unknown>;
    expect(ois.fixed_rate).toBe(0.041);
    expect(ois.receive_fixed).toBe(true);
    expect((ois.periods as unknown[]).length).toBe(5);
    expect((ois.periods as Record<string, unknown>[])[0]).toEqual({ pay: 1, accrual: 1 });

    const pillar = (wire.base_curve as Record<string, unknown>[])[0]!;
    expect(pillar).toEqual({ t: 1, zero_rate: 0.043 });

    const scenario = (wire.scenarios as Record<string, unknown>[])[0]!;
    expect(Object.keys(scenario).sort()).toEqual(
      ["carry_abs", "discount_abs", "rate_shifts", "spot_rel", "vol_abs"].sort(),
    );
    expect(Array.isArray(scenario.rate_shifts)).toBe(true);
    expect((scenario.rate_shifts as number[]).length).toBe(4);
  });

  it("round-trips the request through both codec directions", () => {
    const req = request();
    const back = combinedTailRiskRequestFromWire(combinedTailRiskRequestToWire(req));
    expect(back.optionLegs).toEqual(req.optionLegs);
    expect(back.fiPositions).toEqual(req.fiPositions);
    expect(back.baseCurve).toEqual(req.baseCurve);
    expect(back.scenarios).toEqual(req.scenarios);
    expect(back.alpha).toBe(req.alpha);
  });

  it("encodes + round-trips the response (VarEs + signed key-rate ladder)", () => {
    const res: CombinedTailRiskResponse = {
      jointVarEs: { var: 123456.7, es: 234567.8 },
      keyRate: [
        { tenorYears: 1, dv01: -12.3 },
        { tenorYears: 5, dv01: -456.7 },
      ],
      fiParallelDv01: -987.6,
      correlationId: 42n,
    };
    const wire = combinedTailRiskResponseToWire(res) as Record<string, unknown>;
    expect(wire.joint_var_es).toEqual({ var: 123456.7, es: 234567.8 });
    expect(wire.fi_parallel_dv01).toBe(-987.6);
    expect((wire.key_rate as Record<string, unknown>[])[0]).toEqual({
      tenor_years: 1,
      dv01: -12.3,
    });
    expect(wire.correlation_id).toBe(42);

    expect(combinedTailRiskResponseFromWire(wire)).toEqual(res);
  });
});

// --- (2) offline engine: the joint-tail reduction identities ----------------

describe("combinedTailRiskOffline — joint-tail reduction identities", () => {
  it("options-only ⇒ the options VaR, with NO FI key-rate axis", () => {
    const res = combinedTailRiskOffline(request({ fiPositions: [] }));
    // No FI legs ⇒ no ladder, no parallel DV01 (structural), but a real options tail.
    expect(res.keyRate).toEqual([]);
    expect(res.fiParallelDv01).toBe(0);
    expect(res.jointVarEs.var).toBeGreaterThan(0);
    expect(res.jointVarEs.es).toBeGreaterThanOrEqual(res.jointVarEs.var);
  });

  it("FI-only ⇒ the rate VaR, with a ladder that reconciles to the parallel DV01", () => {
    const res = combinedTailRiskOffline(request({ optionLegs: [] }));
    expect(res.keyRate).toHaveLength(baseCurve().length);
    expect(res.jointVarEs.var).toBeGreaterThan(0);
    // The independent per-pillar 1bp bumps sum (to first order) to the single
    // parallel 1bp bump — the same identity the rates ladder is held to.
    const ladderSum = res.keyRate.reduce((a, k) => a + k.dv01, 0);
    expect(Math.abs(ladderSum - res.fiParallelDv01) / Math.abs(res.fiParallelDv01)).toBeLessThan(1e-3);
    // The tenors are the base-curve pillar times, ascending.
    const tenors = res.keyRate.map((k) => k.tenorYears);
    expect(tenors).toEqual([...tenors].sort((a, b) => a - b));
  });

  it("the FI key-rate axis + parallel DV01 depend ONLY on the FI book (not options / scenarios)", () => {
    const mixed = combinedTailRiskOffline(request());
    const fiOnly = combinedTailRiskOffline(request({ optionLegs: [] }));
    expect(mixed.keyRate).toEqual(fiOnly.keyRate);
    expect(mixed.fiParallelDv01).toBe(fiOnly.fiParallelDv01);
  });

  it("every measure is positively homogeneous — 2× notionals ⇒ 2× VaR/ES/ladder", () => {
    const one = combinedTailRiskOffline(request());
    const scale = (r: CombinedTailRiskRequest): CombinedTailRiskRequest => ({
      ...r,
      optionLegs: r.optionLegs.map((l) => ({ ...l, notionalBase: l.notionalBase * 2 })),
      fiPositions: r.fiPositions.map((p) => ({
        oisSwap: { ...p.oisSwap, notional: p.oisSwap.notional * 2 },
      })),
    });
    const two = combinedTailRiskOffline(scale(request()));
    expect(two.jointVarEs.var).toBeCloseTo(2 * one.jointVarEs.var, 4);
    expect(two.jointVarEs.es).toBeCloseTo(2 * one.jointVarEs.es, 4);
    expect(two.fiParallelDv01).toBeCloseTo(2 * one.fiParallelDv01, 4);
    two.keyRate.forEach((k, i) => {
      expect(k.dv01).toBeCloseTo(2 * one.keyRate[i]!.dv01, 4);
    });
  });

  it("treats alpha = 0 as the 0.99 default (mirrors the server)", () => {
    const withZero = combinedTailRiskOffline(request({ alpha: 0 }));
    const explicit = combinedTailRiskOffline(request({ alpha: 0.99 }));
    expect(withZero.jointVarEs).toEqual(explicit.jointVarEs);
  });

  it("is deterministic and echoes the correlation id", () => {
    const req = request({ correlationId: 7n });
    const a = combinedTailRiskOffline(req);
    const b = combinedTailRiskOffline(req);
    expect(a).toEqual(b);
    expect(a.correlationId).toBe(7n);
  });

  it("rejects a rate shock whose length ≠ the base-curve pillar count (as the server does)", () => {
    const bad = request({
      scenarios: [
        { spotRel: -0.02, volAbs: 0.02, discountAbs: -0.001, carryAbs: 0, rateShifts: [0.001] },
      ],
    });
    expect(() => combinedTailRiskOffline(bad)).toThrow(RatesPricingError);
  });
});

// --- the transport seam + the workspace seed books --------------------------

describe("MockTransport.combinedTailRisk — the offline transport", () => {
  it("rolls up the SAME request the seed books build, joint over options + FI", async () => {
    const t = createMockTransport();
    const curve = defaultTailRiskBaseCurve();
    const res = await t.combinedTailRisk({
      optionLegs: seedTailRiskOptionLegs(EURUSD, MARKET, DEFAULT_CONVENTIONS),
      fiPositions: seedTailRiskFiPositions(),
      baseCurve: curve,
      scenarios: buildJointScenarios(curve.length),
      alpha: 0.99,
    });
    expect(res.jointVarEs.var).toBeGreaterThan(0);
    expect(res.jointVarEs.es).toBeGreaterThanOrEqual(res.jointVarEs.var);
    // One signed key-rate rung per base-curve pillar, reconciling to the parallel DV01.
    expect(res.keyRate).toHaveLength(curve.length);
    const ladderSum = res.keyRate.reduce((a, k) => a + k.dv01, 0);
    expect(Math.abs(ladderSum - res.fiParallelDv01) / Math.abs(res.fiParallelDv01)).toBeLessThan(1e-3);
  });
});
