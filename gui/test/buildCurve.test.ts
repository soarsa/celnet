/**
 * BuildCurve client surface — codec round-trip + offline MockTransport bootstrap.
 *
 * Exercises the REAL `src/data/wsCodec.ts` BuildCurve helpers and the REAL offline
 * `MockTransport.buildCurve` (no server, no mocks-as-real). The encoder produces the
 * snake_case `build_curve` body the server's `crates/celnet-server/src/ws/codec.rs`
 * decodes field-for-field; the decoder reconstructs the camelCase `CalibratedCurve`
 * losslessly. The mock path resolves registry ids, orders the pillars short→long by
 * resolved maturity, and bootstraps the same in-browser discount curve the OIS
 * pricer uses — the numbers are real, the errors are real.
 */
import { describe, expect, it } from "vitest";

import type {
  BuildCurveRequest,
  InstrumentDef,
} from "../src/data/contract";
import { MockTransport } from "../src/data/mockSource";
import {
  buildCurveRequestToWire,
  calibratedCurveFromWire,
  type WireObject,
} from "../src/data/wsCodec";

const REQUEST: BuildCurveRequest = {
  requestId: "curve-001",
  currency: "USD",
  referenceDate: { year: 2026, month: 6, day: 25 },
  pillars: [
    { instrumentId: "usd-depo-3m", quote: 0.0431 },
    { instrumentId: "usd-irs-2y", quote: 0.0405 },
    { instrumentId: "usd-irs-10y", quote: 0.0418 },
  ],
  datePillars: [],
};

/** A USD deposit/IRS roster to calibrate against in the offline path. */
function usdDeposit(id: string, tenor: string): InstrumentDef {
  return {
    instrumentId: id,
    name: id,
    description: "",
    currency: "USD",
    externalIds: [],
    family: "deposit",
    deposit: {
      index: "SOFR",
      tenor,
      dayCount: "act_360",
      businessDayConvention: "modified_following",
      calendars: ["united_states"],
      spotLagDays: 2,
    },
  };
}

function usdIrs(id: string, tenor: string): InstrumentDef {
  return {
    instrumentId: id,
    name: id,
    description: "",
    currency: "USD",
    externalIds: [],
    family: "vanilla_irs",
    vanillaIrs: {
      tenor,
      fixedFrequency: "annual",
      fixedDayCount: "act_360",
      floatIndex: "SOFR",
      floatFrequency: "annual",
      floatDayCount: "act_360",
      businessDayConvention: "modified_following",
      calendars: ["united_states"],
      rollConvention: "none",
      spotLagDays: 2,
    },
  };
}

describe("wsCodec — buildCurveRequestToWire", () => {
  it("encodes the snake_case build_curve body (no auth fields — connection injects them)", () => {
    const wire = buildCurveRequestToWire(REQUEST);
    expect(wire).toEqual({
      request_id: "curve-001",
      currency: "USD",
      reference_date: { year: 2026, month: 6, day: 25 },
      pillars: [
        { instrument_id: "usd-depo-3m", quote: 0.0431 },
        { instrument_id: "usd-irs-2y", quote: 0.0405 },
        { instrument_id: "usd-irs-10y", quote: 0.0418 },
      ],
      date_pillars: [],
    });
    // The bearer token + correlation id are injected by WsConnection.request.
    expect("session_token" in wire).toBe(false);
    expect("correlation_id" in wire).toBe(false);
  });
});

describe("wsCodec — calibratedCurveFromWire", () => {
  it("decodes a server-shaped calibrated_curve frame losslessly", () => {
    const frame: WireObject = {
      request_id: "curve-001",
      currency: "USD",
      reference_date: { year: 2026, month: 6, day: 25 },
      points: [
        {
          instrument_id: "usd-depo-3m",
          time_years: 0.2521,
          discount_factor: 0.98912,
          zero_rate: 0.04318,
          label: "",
        },
        {
          instrument_id: "",
          time_years: 1.5151,
          discount_factor: 0.93827,
          zero_rate: 0.0415,
          label: "Date 2027-12-31",
        },
        {
          instrument_id: "usd-irs-10y",
          time_years: 10.0,
          discount_factor: 0.6612,
          zero_rate: 0.04134,
          label: "",
        },
      ],
    };
    const curve = calibratedCurveFromWire(frame);
    expect(curve.requestId).toBe("curve-001");
    expect(curve.currency).toBe("USD");
    expect(curve.referenceDate).toEqual({ year: 2026, month: 6, day: 25 });
    expect(curve.points).toEqual([
      {
        instrumentId: "usd-depo-3m",
        timeYears: 0.2521,
        discountFactor: 0.98912,
        zeroRate: 0.04318,
        label: "",
      },
      {
        instrumentId: "",
        timeYears: 1.5151,
        discountFactor: 0.93827,
        zeroRate: 0.0415,
        label: "Date 2027-12-31",
      },
      {
        instrumentId: "usd-irs-10y",
        timeYears: 10.0,
        discountFactor: 0.6612,
        zeroRate: 0.04134,
        label: "",
      },
    ]);
  });

  it("round-trips the request id / currency / reference date through the wire", () => {
    const wire = buildCurveRequestToWire(REQUEST);
    // The server echoes request_id / currency / reference_date onto the result;
    // emulate that header and confirm the decoder recovers exactly what we sent.
    const echoed: WireObject = {
      request_id: wire.request_id,
      currency: wire.currency,
      reference_date: wire.reference_date,
      points: [],
    };
    const curve = calibratedCurveFromWire(echoed);
    expect(curve.requestId).toBe(REQUEST.requestId);
    expect(curve.currency).toBe(REQUEST.currency);
    expect(curve.referenceDate).toEqual(REQUEST.referenceDate);
  });
});

describe("MockTransport.buildCurve — offline bootstrap from registry instruments", () => {
  async function seeded(): Promise<MockTransport> {
    const t = new MockTransport();
    await t.createInstrument(usdDeposit("usd-depo-3m", "3M"));
    await t.createInstrument(usdIrs("usd-irs-10y", "10Y"));
    await t.createInstrument(usdIrs("usd-irs-2y", "2Y"));
    return t;
  }

  it("resolves ids, orders short→long, and returns a monotone discount curve", async () => {
    const t = await seeded();
    const curve = await t.buildCurve(REQUEST);

    expect(curve.requestId).toBe("curve-001");
    expect(curve.currency).toBe("USD");
    expect(curve.points).toHaveLength(3);

    // Ordered short→long by resolved maturity, regardless of request order.
    const times = curve.points.map((p) => p.timeYears);
    expect([...times]).toEqual([...times].sort((a, b) => a - b));
    expect(curve.points[0]!.instrumentId).toBe("usd-depo-3m");
    expect(curve.points[2]!.instrumentId).toBe("usd-irs-10y");

    // Discount factors are in (0, 1] and strictly decrease with maturity.
    for (const p of curve.points) {
      expect(p.discountFactor).toBeGreaterThan(0);
      expect(p.discountFactor).toBeLessThanOrEqual(1);
      expect(Number.isFinite(p.zeroRate)).toBe(true);
    }
    for (let i = 1; i < curve.points.length; i++) {
      expect(curve.points[i]!.discountFactor).toBeLessThan(
        curve.points[i - 1]!.discountFactor,
      );
    }
  });

  it("rejects an unresolved instrument id", async () => {
    const t = await seeded();
    await expect(
      t.buildCurve({
        ...REQUEST,
        pillars: [{ instrumentId: "usd-irs-30y", quote: 0.042 }],
      }),
    ).rejects.toThrow(/no instrument with id/);
  });

  it("rejects a currency mismatch against the curve currency", async () => {
    const t = await seeded();
    await t.createInstrument({ ...usdIrs("eur-irs-5y", "5Y"), currency: "EUR" });
    await expect(
      t.buildCurve({
        ...REQUEST,
        pillars: [{ instrumentId: "eur-irs-5y", quote: 0.027 }],
      }),
    ).rejects.toThrow(/not the curve currency/);
  });

  it("rejects an empty pillar set", async () => {
    const t = await seeded();
    await expect(
      t.buildCurve({ ...REQUEST, pillars: [], datePillars: [] }),
    ).rejects.toThrow(/at least one/);
  });

  it("calibrates a standalone date-anchored pillar to a closed-form deposit", async () => {
    const t = await seeded();
    const curve = await t.buildCurve({
      ...REQUEST,
      pillars: [],
      datePillars: [
        { maturityDate: { year: 2027, month: 12, day: 31 }, quote: 0.0415 },
      ],
    });

    expect(curve.points).toHaveLength(1);
    const p = curve.points[0]!;
    // A date pillar has no instrument id and a `Date YYYY-MM-DD` label.
    expect(p.instrumentId).toBe("");
    expect(p.label).toBe("Date 2027-12-31");
    // DF = 1/(1 + r·τ) with ACT/360 accrual ⇒ within (0, 1); zero rate finite.
    const tau360 = (p.timeYears * 365) / 360;
    expect(p.discountFactor).toBeCloseTo(1 / (1 + 0.0415 * tau360), 12);
    expect(p.discountFactor).toBeGreaterThan(0);
    expect(p.discountFactor).toBeLessThan(1);
    expect(Number.isFinite(p.zeroRate)).toBe(true);
  });

  it("merges instrument and date pillars, ordered by maturity", async () => {
    const t = await seeded();
    const curve = await t.buildCurve({
      ...REQUEST,
      datePillars: [
        { maturityDate: { year: 2027, month: 12, day: 31 }, quote: 0.0415 },
      ],
    });

    // Three instrument pillars + one date pillar, sorted short→long by maturity.
    expect(curve.points).toHaveLength(4);
    const times = curve.points.map((pt) => pt.timeYears);
    expect([...times]).toEqual([...times].sort((a, b) => a - b));
    const datePoint = curve.points.find((pt) => pt.instrumentId === "");
    expect(datePoint?.label).toBe("Date 2027-12-31");
  });
});
