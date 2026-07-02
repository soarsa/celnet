import { createElement } from "react";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { RatesCurveSet } from "../src/data/contract";
import {
  bootstrapCurveFromSet,
  DEFAULT_USD_SOFR_CURVE,
  discountFactorAt,
  instantaneousForwardAt,
  RatesPricingError,
  sampleCurve,
  zeroRateAt,
} from "../src/data/ratesPricing";
import { CurveWorkspace, curvePillarNodes } from "../src/workspaces/CurveWorkspace";

/**
 * Curve-inspection identities for the Curve workspace (FI-ARCHITECTURE §4.2). We
 * assert the structural properties a real bootstrapped discount curve must satisfy
 * — `DF(0) = 1` exactly, `DF` strictly decreasing in `t`, the continuously-
 * compounded zero rate reproducing the calibrating par-OIS pillars to a few bp, and
 * the instantaneous forward sitting at the zero-rate level — rather than pinning
 * opaque magic numbers. The sampling functions are thin views over the SAME
 * `bootstrapOis` math the `ratesPricing` pricer (and the live edge) use, so a curve
 * that inspects cleanly is the curve that prices.
 */

const PILLAR_TENORS = DEFAULT_USD_SOFR_CURVE.pillars.map((p) => p.tenorYears);

describe("sampleCurve — discount-curve identities", () => {
  it("anchors the discount factor to exactly 1 at the reference date", () => {
    const [origin] = sampleCurve(DEFAULT_USD_SOFR_CURVE, { samples: 64 });
    expect(origin!.t).toBe(0);
    expect(origin!.df).toBe(1);
  });

  it("returns the requested number of points across the curve span", () => {
    const span = PILLAR_TENORS[PILLAR_TENORS.length - 1]!;
    const points = sampleCurve(DEFAULT_USD_SOFR_CURVE, { samples: 50 });
    expect(points.length).toBe(50);
    expect(points[0]!.t).toBe(0);
    expect(points[points.length - 1]!.t).toBeCloseTo(span, 12);
  });

  it("discounts strictly monotonically — DF falls as t rises", () => {
    const points = sampleCurve(DEFAULT_USD_SOFR_CURVE, { samples: 240 });
    for (let i = 1; i < points.length; i += 1) {
      expect(points[i]!.df).toBeLessThan(points[i - 1]!.df);
    }
  });

  it("keeps the zero rate and the forward in a positive, sane band the length of the curve", () => {
    const points = sampleCurve(DEFAULT_USD_SOFR_CURVE, { samples: 240 });
    for (const p of points) {
      expect(Number.isFinite(p.zero)).toBe(true);
      expect(Number.isFinite(p.forward)).toBe(true);
      expect(p.zero).toBeGreaterThan(0.02);
      expect(p.zero).toBeLessThan(0.07);
      expect(p.forward).toBeGreaterThan(0.02);
      expect(p.forward).toBeLessThan(0.07);
    }
  });

  it("reproduces each calibrating par-OIS pillar in the zero rate to a few bp", () => {
    // The continuously-compounded zero at a pillar tenor is close to — but not
    // identical to — the annual-compounded par swap rate calibrated there; the
    // difference is the compounding/annuity convexity, a handful of bp on this
    // near-flat curve. We reconcile on a 20 bp tolerance.
    const curve = bootstrapCurveFromSet(DEFAULT_USD_SOFR_CURVE);
    for (const pillar of DEFAULT_USD_SOFR_CURVE.pillars) {
      const zero = zeroRateAt(curve, pillar.tenorYears);
      expect(Math.abs(zero - pillar.parRate)).toBeLessThan(0.0020);
    }
  });

  it("places the instantaneous forward at the zero-rate level (within curvature)", () => {
    // For a smooth, near-flat curve the forward tracks the zero rate; they diverge
    // only by the slope of the zero curve (mild here), so reconcile within 60 bp.
    const curve = bootstrapCurveFromSet(DEFAULT_USD_SOFR_CURVE);
    for (const tenor of PILLAR_TENORS) {
      const zero = zeroRateAt(curve, tenor);
      const forward = instantaneousForwardAt(curve, tenor);
      expect(Math.abs(forward - zero)).toBeLessThan(0.0060);
    }
  });

  it("agrees point-for-point with the discount curve it bootstraps", () => {
    // sampleCurve must be a pure view over bootstrapCurveFromSet + the *At readers.
    const curve = bootstrapCurveFromSet(DEFAULT_USD_SOFR_CURVE);
    const points = sampleCurve(DEFAULT_USD_SOFR_CURVE, { samples: 33 });
    for (const p of points) {
      expect(p.df).toBe(discountFactorAt(curve, p.t));
      expect(p.zero).toBe(zeroRateAt(curve, p.t));
      expect(p.forward).toBe(instantaneousForwardAt(curve, p.t));
    }
  });

  it("samples to an explicit shorter horizon when asked", () => {
    const points = sampleCurve(DEFAULT_USD_SOFR_CURVE, { samples: 12, maxTenor: 5 });
    expect(points[points.length - 1]!.t).toBeCloseTo(5, 12);
  });
});

describe("zeroRateAt — the t → 0 short-rate limit", () => {
  it("continues the zero rate to the instantaneous short rate at the origin", () => {
    const curve = bootstrapCurveFromSet(DEFAULT_USD_SOFR_CURVE);
    // l'Hôpital: z(0) = f(0). The origin is not a 0/0 singularity.
    expect(zeroRateAt(curve, 0)).toBe(instantaneousForwardAt(curve, 0));
    expect(zeroRateAt(curve, 0)).toBeGreaterThan(0.02);
  });
});

describe("bootstrapCurveFromSet — rejects a malformed curve exactly as the pricer does", () => {
  const reject = (curve: RatesCurveSet) => () => bootstrapCurveFromSet(curve);

  it("rejects an unsupported currency", () => {
    expect(reject({ ...DEFAULT_USD_SOFR_CURVE, currency: "EUR" })).toThrow(RatesPricingError);
  });

  it("rejects an empty pillar set", () => {
    expect(reject({ ...DEFAULT_USD_SOFR_CURVE, pillars: [] })).toThrow(RatesPricingError);
  });

  it("rejects non-strictly-increasing pillar tenors", () => {
    const curve: RatesCurveSet = {
      ...DEFAULT_USD_SOFR_CURVE,
      pillars: [
        { tenorYears: 3, parRate: 0.04 },
        { tenorYears: 3, parRate: 0.041 },
      ],
    };
    expect(reject(curve)).toThrow(RatesPricingError);
  });
});

describe("curvePillarNodes — the YieldCurve wiring off the real bootstrap", () => {
  const curve = bootstrapCurveFromSet(DEFAULT_USD_SOFR_CURVE);
  const ladder = DEFAULT_USD_SOFR_CURVE.pillars.map((p) => ({
    tenorYears: p.tenorYears,
    zero: zeroRateAt(curve, p.tenorYears),
  }));

  it("maps one dated node per pillar, carrying the bootstrapped zero rate", () => {
    const nodes = curvePillarNodes(ladder);
    expect(nodes.length).toBe(DEFAULT_USD_SOFR_CURVE.pillars.length);
    for (let i = 0; i < nodes.length; i += 1) {
      expect(nodes[i]!.label).toBe(`${ladder[i]!.tenorYears}y`);
      expect(nodes[i]!.tenorYears).toBe(ladder[i]!.tenorYears);
      expect(nodes[i]!.zeroRate).toBe(ladder[i]!.zero);
    }
  });

  it("feeds nodes whose ln DF reconstruction reproduces the real discount factors", () => {
    // YieldCurve rebuilds ln DF(t_i) = −zeroRate·tenorYears from each node; that
    // must round-trip to the SAME discount factor the workspace bootstrap produced,
    // so the drawn curve is the curve that prices.
    for (const node of curvePillarNodes(ladder)) {
      const reconstructedDf = Math.exp(-node.zeroRate * node.tenorYears);
      expect(reconstructedDf).toBeCloseTo(discountFactorAt(curve, node.tenorYears), 12);
    }
  });
});

describe("CurveWorkspace — renders the YieldCurve term structure", () => {
  it("mounts the YieldCurve chart with its zero / forward / DF overlay legend", () => {
    render(createElement(CurveWorkspace));
    // The three per-overlay legend toggles are the YieldCurve's own controls —
    // their presence proves the workspace wired real (≥2-pillar) nodes into it,
    // since the chart renders an explicit empty state otherwise.
    expect(screen.getByRole("button", { name: /^Zero/ })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: /^Fwd/ })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: /^DF/ })).toHaveAttribute("aria-pressed", "true");
    expect(
      screen.queryByRole("img", { name: /yield curve unavailable/i }),
    ).toBeNull();
  });
});
