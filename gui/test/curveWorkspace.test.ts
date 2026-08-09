import { createElement } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { CurveDefinition, RatesCurveSet } from "../src/data/contract";
import { yearsPillarTenor } from "../src/data/contract";
import {
  bootstrapCurveFromSet,
  DEFAULT_USD_SOFR_CURVE,
  discountFactorAt,
  instantaneousForwardAt,
  pillarMaturityYears,
  RatesPricingError,
  sampleCurve,
  zeroRateAt,
} from "../src/data/ratesPricing";

// The manager is now app-driven (it lists / mutates curve definitions through the one
// contract), so the UI-render blocks below mock `useApp` with a fixture transport +
// auth, exactly as the other workspace suites do. The pure-math blocks import the
// same module but never touch the mock.
const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { CurveWorkspace, curvePillarNodes } from "../src/workspaces/CurveWorkspace";

/** The two seeded curve definitions the fixture transport lists (usd-sofr primary). */
function seededDefs(): CurveDefinition[] {
  const base = {
    indexLabel: "USD-SOFR",
    dayCount: "ACT/360",
    calendar: "USD",
    pillars: structuredClone(DEFAULT_USD_SOFR_CURVE),
  };
  return [
    { ...base, curveId: "usd-sofr", displayName: "USD SOFR", interpolation: "log-linear-df", primary: true, pillars: structuredClone(DEFAULT_USD_SOFR_CURVE) },
    { ...base, curveId: "usd-sofr-street", displayName: "USD SOFR (street)", interpolation: "monotone-convex-forward", primary: false, pillars: structuredClone(DEFAULT_USD_SOFR_CURVE) },
  ];
}

/** A mock app whose transport lists the seeded curves; `canEdit` gates the `refdata` cap. */
function makeApp(opts: { canEdit?: boolean } = {}) {
  const defs = seededDefs();
  const updateCurveDefinition = vi.fn(async (_id: string, d: CurveDefinition) => d);
  const transport = {
    listCurveDefinitions: vi.fn(async () => defs.map((d) => structuredClone(d))),
    createCurveDefinition: vi.fn(async (d: CurveDefinition) => d),
    updateCurveDefinition,
    deleteCurveDefinition: vi.fn(async () => {}),
  };
  const canEdit = opts.canEdit ?? true;
  return {
    app: {
      transport,
      auth: {
        user: { id: "u", email: "admin@celnet.com" },
        isAdmin: true,
        can: (action: string) => (action === "refdata" ? canEdit : true),
      },
    },
    transport,
    updateCurveDefinition,
  };
}

/** Render the manager and wait for the seeded list to resolve into the dashboard. */
async function renderManager(app: unknown): Promise<void> {
  state.app = app;
  render(createElement(CurveWorkspace));
  await screen.findByText("USD SOFR");
}

/** The reference (spot) date the DEFAULT curve's pillar schedules roll from. */
const REF = DEFAULT_USD_SOFR_CURVE.referenceDate;

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

const PILLAR_TENORS = DEFAULT_USD_SOFR_CURVE.pillars.map((p) =>
  pillarMaturityYears(p.tenor, REF),
);

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
      const zero = zeroRateAt(curve, pillarMaturityYears(pillar.tenor, REF));
      expect(Math.abs(zero - pillar.parRate)).toBeLessThan(0.002);
    }
  });

  it("places the instantaneous forward at the zero-rate level (within curvature)", () => {
    // For a smooth, near-flat curve the forward tracks the zero rate; they diverge
    // only by the slope of the zero curve (mild here), so reconcile within 60 bp.
    const curve = bootstrapCurveFromSet(DEFAULT_USD_SOFR_CURVE);
    for (const tenor of PILLAR_TENORS) {
      const zero = zeroRateAt(curve, tenor);
      const forward = instantaneousForwardAt(curve, tenor);
      expect(Math.abs(forward - zero)).toBeLessThan(0.006);
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
    const points = sampleCurve(DEFAULT_USD_SOFR_CURVE, {
      samples: 12,
      maxTenor: 5,
    });
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
    expect(reject({ ...DEFAULT_USD_SOFR_CURVE, currency: "EUR" })).toThrow(
      RatesPricingError,
    );
  });

  it("rejects an empty pillar set", () => {
    expect(reject({ ...DEFAULT_USD_SOFR_CURVE, pillars: [] })).toThrow(
      RatesPricingError,
    );
  });

  it("rejects non-strictly-increasing pillar tenors", () => {
    const curve: RatesCurveSet = {
      ...DEFAULT_USD_SOFR_CURVE,
      pillars: [
        { tenor: yearsPillarTenor(3), parRate: 0.04 },
        { tenor: yearsPillarTenor(3), parRate: 0.041 },
      ],
    };
    expect(reject(curve)).toThrow(RatesPricingError);
  });
});

describe("curvePillarNodes — the YieldCurve wiring off the real bootstrap", () => {
  const curve = bootstrapCurveFromSet(DEFAULT_USD_SOFR_CURVE);
  // Each pillar's curve-time is its `PillarTenor` resolved to a year-fraction from
  // the reference (spot) date — the SAME coordinate the workspace places it at.
  const ladder = DEFAULT_USD_SOFR_CURVE.pillars.map((p) => {
    const tenorYears = pillarMaturityYears(p.tenor, REF);
    return { tenorYears, zero: zeroRateAt(curve, tenorYears) };
  });

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

describe("CurveWorkspace — the multi-curve manager dashboard", () => {
  it("lists every persisted curve with its per-currency primary badge", async () => {
    await renderManager(makeApp().app);
    // Both seeded curves are listed by display name…
    expect(screen.getByText("USD SOFR")).toBeInTheDocument();
    expect(screen.getByText("USD SOFR (street)")).toBeInTheDocument();
    // …and exactly one carries the Primary badge (usd-sofr).
    const badges = screen.getAllByText("Primary");
    expect(badges.length).toBe(1);
    // The interpolation of each curve is shown (log-linear + monotone-convex).
    expect(screen.getByText(/Log-linear \(DF\)/)).toBeInTheDocument();
    expect(screen.getByText(/Monotone convex \(forward\)/)).toBeInTheDocument();
  });

  it("exposes New curve + Delete only to a Refdata·FI editor (read-only viewer sees neither)", async () => {
    await renderManager(makeApp({ canEdit: true }).app);
    expect(screen.getByRole("button", { name: /new curve/i })).toBeInTheDocument();
    // The non-primary curve's Delete is offered to an editor.
    expect(screen.getAllByRole("button", { name: /^Delete$/ }).length).toBeGreaterThan(0);

    cleanup();
    await renderManager(makeApp({ canEdit: false }).app);
    // A read-only viewer still SEES the dashboard, but no write affordances.
    expect(screen.getByText("USD SOFR")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /new curve/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /^Delete$/ })).toBeNull();
  });
});

describe("CurveWorkspace — interpolation is now wire-real + selectable", () => {
  it("offers BOTH schemes enabled in the definition editor; the selected curve's is checked", async () => {
    await renderManager(makeApp().app);
    // Open the Definition tab — it edits the selected (primary) curve, whose scheme
    // is log-linear DF.
    fireEvent.click(screen.getByRole("tab", { name: /^Definition$/ }));

    const logLinear = screen.getByRole("radio", { name: /log-linear \(df\)/i });
    const monotone = screen.getByRole("radio", { name: /monotone convex/i });
    // Neither is disabled any more — the interpolation rides the wire (38bcff9a).
    expect(logLinear).toBeEnabled();
    expect(monotone).toBeEnabled();
    expect(logLinear).toBeChecked();
    expect(monotone).not.toBeChecked();

    // Selecting monotone-convex is honoured (no fabricated "Target" affordance).
    fireEvent.click(monotone);
    expect(monotone).toBeChecked();
    expect(logLinear).not.toBeChecked();
  });
});

describe("CurveWorkspace — the per-curve pillar lens off the selected curve", () => {
  it("selects the active curve and renders the YieldCurve term structure", async () => {
    await renderManager(makeApp().app);
    fireEvent.click(screen.getByRole("tab", { name: /^Pillars$/ }));

    // The active-curve picker drives the lens (the seeded curves are its options).
    expect(screen.getByRole("combobox", { name: /active curve/i })).toBeInTheDocument();

    // The YieldCurve's own overlay toggles prove real (≥2-pillar) nodes were wired in.
    expect(screen.getByRole("button", { name: /^Zero/ })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: /^Fwd/ })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: /^DF/ })).toHaveAttribute("aria-pressed", "true");
    expect(
      screen.queryByRole("img", { name: /yield curve unavailable/i }),
    ).toBeNull();
  });

  it("saves edited pillars back to the selected curve via update_curve_definition", async () => {
    const h = makeApp();
    await renderManager(h.app);
    fireEvent.click(screen.getByRole("tab", { name: /^Pillars$/ }));

    // Edit the first pillar's par rate, then Save.
    const firstRate = screen.getAllByRole("spinbutton", { name: /par rate in percent/i })[0]!;
    fireEvent.change(firstRate, { target: { value: "4.5" } });
    fireEvent.click(screen.getByRole("button", { name: /save pillars/i }));

    await screen.findByText(/^Saved$/);
    // The update rode the one contract, keyed by the selected (primary) curve id.
    expect(h.updateCurveDefinition).toHaveBeenCalledWith(
      "usd-sofr",
      expect.objectContaining({ curveId: "usd-sofr" }),
    );
  });
});
