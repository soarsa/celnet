import { describe, expect, it } from "vitest";
import {
  DEFAULT_CONVENTIONS,
  ShapingError,
  formatCalibratedSmileSpill,
  formatSeriesCell,
  parseDeltaWing,
  parseObservable,
  parseSmileModel,
  shapeCalibration,
} from "../src/functions/shaping";
import { marketObservable, smileModel } from "../src/contract/enums";
import { SeriesRegistry, seriesKey } from "../src/functions/seriesRegistry";
import type { MarketSeriesRequest, StreamEvent } from "../src/transport/connection";
import type { SeriesTick } from "../src/functions/seriesRegistry";
import type { SpillMatrix } from "../src/functions/shaping";

/** Read a spill cell with a presence assertion (strict `noUncheckedIndexedAccess`). */
function cell(spill: SpillMatrix, row: number, col: number): string | number {
  const r = spill[row];
  expect(r).toBeDefined();
  const v = r![col];
  expect(v).toBeDefined();
  return v!;
}

// ---------------------------------------------------------------------------
// smile-model + observable parsing (the new selector vocabulary)
// ---------------------------------------------------------------------------

describe("parseSmileModel", () => {
  it("maps trader short names and canonical names to the contract model", () => {
    expect(parseSmileModel(undefined)).toBe("MARKET_HEDGE");
    expect(parseSmileModel("")).toBe("MARKET_HEDGE");
    expect(parseSmileModel("vv")).toBe("MARKET_HEDGE");
    expect(parseSmileModel("SABR")).toBe("STOCHASTIC_VOL");
    expect(parseSmileModel("svi")).toBe("PARAMETRIC");
    expect(parseSmileModel("SSVI")).toBe("PARAMETRIC_SURFACE");
    expect(parseSmileModel("parametric_surface")).toBe("PARAMETRIC_SURFACE");
  });
  it("accepts the eSSVI / extended-surface aliases (case-insensitive)", () => {
    expect(parseSmileModel("ESSVI")).toBe("EXTENDED_SURFACE");
    expect(parseSmileModel("essvi")).toBe("EXTENDED_SURFACE");
    expect(parseSmileModel("Extended")).toBe("EXTENDED_SURFACE");
    expect(parseSmileModel("EXTENDED")).toBe("EXTENDED_SURFACE");
    expect(parseSmileModel("extended_surface")).toBe("EXTENDED_SURFACE");
    expect(parseSmileModel("EXTENDED-SURFACE")).toBe("EXTENDED_SURFACE");
  });
  it("rejects an unknown model", () => {
    expect(() => parseSmileModel("HESTON")).toThrow(ShapingError);
  });
  it("the rejection message is generated from the enum member list (cannot drift)", () => {
    // The accepted-list message must name EXTENDED_SURFACE — derived from
    // SMILE_MODEL_MEMBERS, so adding a model can never leave the message stale.
    expect(() => parseSmileModel("HESTON")).toThrow(/EXTENDED_SURFACE/);
    expect(() => parseSmileModel("HESTON")).toThrow(/ESSVI/);
  });
  it("projects to the exact proto enum number and back", () => {
    for (const m of [
      "MARKET_HEDGE",
      "STOCHASTIC_VOL",
      "PARAMETRIC",
      "PARAMETRIC_SURFACE",
      "EXTENDED_SURFACE",
    ] as const) {
      expect(smileModel.fromWire(smileModel.toWire(m))).toBe(m);
    }
    // The wire numbers ARE the proto tags (SMILE_MODEL_MARKET_HEDGE=0, …=4).
    expect(smileModel.toWire("MARKET_HEDGE")).toBe(0);
    expect(smileModel.toWire("STOCHASTIC_VOL")).toBe(1);
    expect(smileModel.toWire("PARAMETRIC")).toBe(2);
    expect(smileModel.toWire("PARAMETRIC_SURFACE")).toBe(3);
    expect(smileModel.toWire("EXTENDED_SURFACE")).toBe(4);
  });
  it("CELNET.MARKSURFACE(...,\"ESSVI\") routes EXTENDED_SURFACE (proto 4) on the wire", () => {
    // The full mark route: a trader-typed "ESSVI" model arg shapes the calibration
    // (shapeCalibration → parseSmileModel) and the function layer encodes
    // smile_model = smileModel.toWire(shaped.model) onto the wire body.
    const shaped = shapeCalibration({
      pair: "EURUSD",
      tenor: "1Y",
      model: "ESSVI",
      atmVol: 0.102,
      rr25: 0.001,
      bf25: 0.002,
    });
    expect(shaped.model).toBe("EXTENDED_SURFACE");
    expect(smileModel.toWire(shaped.model)).toBe(4);
  });
});

describe("parseObservable", () => {
  it("maps trader names to the contract observable", () => {
    expect(parseObservable("ATM")).toBe("ATM_VOL");
    expect(parseObservable("spot")).toBe("SPOT");
    expect(parseObservable("RR")).toBe("RISK_REVERSAL");
    expect(parseObservable("bf")).toBe("BUTTERFLY");
    expect(parseObservable("FWD")).toBe("FORWARD");
  });
  it("rejects an unknown observable", () => {
    expect(() => parseObservable("VANNA")).toThrow(ShapingError);
  });
  it("projects to the exact proto enum number (ATM_VOL=0,…,FORWARD=4)", () => {
    expect(marketObservable.toWire("ATM_VOL")).toBe(0);
    expect(marketObservable.toWire("SPOT")).toBe(1);
    expect(marketObservable.toWire("RISK_REVERSAL")).toBe(2);
    expect(marketObservable.toWire("BUTTERFLY")).toBe(3);
    expect(marketObservable.toWire("FORWARD")).toBe(4);
  });
});

describe("parseDeltaWing", () => {
  it("accepts a fraction or a percent-delta and normalizes to a fraction", () => {
    expect(parseDeltaWing(0.25)).toBeCloseTo(0.25, 12);
    expect(parseDeltaWing("25")).toBeCloseTo(0.25, 12);
    expect(parseDeltaWing("10d")).toBeCloseTo(0.1, 12);
  });
  it("rejects an out-of-range wing", () => {
    expect(() => parseDeltaWing(0)).toThrow(ShapingError);
    expect(() => parseDeltaWing(0.6)).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// calibration request shaping (CELNET.MARKSURFACE)
// ---------------------------------------------------------------------------

describe("shapeCalibration", () => {
  it("shapes a 3-point mark (25Δ only) under a chosen model", () => {
    const c = shapeCalibration({
      pair: "eurusd",
      tenor: "1Y",
      model: "SABR",
      atmVol: 0.102,
      rr25: 0.01,
      bf25: 0.003,
    });
    expect(c.pair).toEqual({ base: "EUR", quote: "USD" });
    expect(c.tenorYears).toBeCloseTo(1, 12);
    expect(c.model).toBe("STOCHASTIC_VOL");
    expect(c.hasTenDelta).toBe(false);
    expect(c.rr10).toBe(0);
    expect(c.bf10).toBe(0);
  });
  it("shapes a 5-point mark when both 10Δ wings are supplied", () => {
    const c = shapeCalibration({
      pair: "EURUSD",
      tenor: "3M",
      model: "SVI",
      atmVol: 0.105,
      rr25: 0.012,
      bf25: 0.0035,
      rr10: 0.02,
      bf10: 0.008,
    });
    expect(c.model).toBe("PARAMETRIC");
    expect(c.hasTenDelta).toBe(true);
    expect(c.rr10).toBeCloseTo(0.02, 12);
    expect(c.bf10).toBeCloseTo(0.008, 12);
  });
  it("rejects a non-finite ATM vol or bad model", () => {
    expect(() => shapeCalibration({ pair: "EURUSD", tenor: "1Y", model: "VV", atmVol: 0, rr25: 0, bf25: 0 })).toThrow(
      ShapingError,
    );
    expect(() =>
      shapeCalibration({ pair: "EURUSD", tenor: "1Y", model: "NOPE", atmVol: 0.1, rr25: 0, bf25: 0 }),
    ).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// formatting: calibrated-smile spill + series cell
// ---------------------------------------------------------------------------

describe("formatCalibratedSmileSpill", () => {
  const points = [
    { delta: -0.25, vol: 0.108 },
    { delta: 0, vol: 0.102 },
    { delta: 0.25, vol: 0.112 },
  ];
  it("spills delta header + vol row + a model/version footer", () => {
    const spill = formatCalibratedSmileSpill({
      points,
      requestedModel: "STOCHASTIC_VOL",
      providerNote: "calibrated; model=stochastic_vol",
      arbFree: true,
      conv: DEFAULT_CONVENTIONS,
      surfaceVersion: 7n,
      epochNanos: 0n,
    });
    expect(spill).toHaveLength(3);
    expect(cell(spill, 0, 0)).toBe("delta");
    expect(cell(spill, 1, 0)).toBe("vol");
    // Sorted ascending by signed delta (put … call).
    expect(spill[0]!.slice(1)).toEqual([-0.25, 0, 0.25]);
    const footer = String(cell(spill, 2, 0));
    // Provenance from the server note is preferred and surfaced verbatim.
    expect(footer).toMatch(/model stochastic_vol/);
    expect(footer).toMatch(/arb-free/);
    expect(footer).toMatch(/surface v7/);
  });
  it("falls back to the requested model when the note carries no provenance", () => {
    const spill = formatCalibratedSmileSpill({
      points,
      requestedModel: "PARAMETRIC",
      providerNote: "no provenance token here",
      arbFree: false,
      conv: DEFAULT_CONVENTIONS,
      surfaceVersion: undefined,
      epochNanos: 0n,
    });
    const footer = String(cell(spill, 2, 0));
    expect(footer).toMatch(/model PARAMETRIC/);
    expect(footer).toMatch(/ARB!/);
  });
});

describe("formatSeriesCell", () => {
  it("renders a vol observable in vol points and a rate in full precision", () => {
    expect(formatSeriesCell({ value: 0.1023, observable: "ATM_VOL", baselined: true, epochNanos: 1n })).toBe("10.23v");
    expect(formatSeriesCell({ value: 0.02, observable: "RISK_REVERSAL", baselined: true, epochNanos: 1n })).toBe("2.00v");
    expect(formatSeriesCell({ value: 1.10355, observable: "SPOT", baselined: true, epochNanos: 1n })).toBe("1.10355");
    expect(formatSeriesCell({ value: 1.1098, observable: "FORWARD", baselined: true, epochNanos: 1n })).toBe("1.10980");
  });
  it("shows a waiting marker before a baseline / for a non-finite value", () => {
    expect(formatSeriesCell({ value: Number.NaN, observable: "SPOT", baselined: false, epochNanos: 0n })).toBe(
      "… (awaiting)",
    );
    expect(formatSeriesCell({ value: Number.NaN, observable: "ATM_VOL", baselined: true, epochNanos: 1n })).toBe(
      "… (awaiting)",
    );
  });
});

// ---------------------------------------------------------------------------
// series registry: dedup, refcount, latest-tick, clean teardown
// ---------------------------------------------------------------------------

/** A tiny in-memory connection fake that drives the registry deterministically. */
class FakeSeriesConnection {
  private listener: ((e: StreamEvent) => void) | null = null;
  private nextId = 1n;
  public readonly opened: { id: bigint; req: MarketSeriesRequest }[] = [];
  public readonly closed: bigint[] = [];

  subscribeSeries(req: MarketSeriesRequest): bigint {
    const id = this.nextId++;
    this.opened.push({ id, req });
    return id;
  }
  unsubscribeSeries(id: bigint): void {
    this.closed.push(id);
  }
  onEvent(listener: (e: StreamEvent) => void): () => void {
    this.listener = listener;
    return () => {
      this.listener = null;
    };
  }
  emit(e: StreamEvent): void {
    this.listener?.(e);
  }
}

const REQ: MarketSeriesRequest = {
  pair: { base: "EUR", quote: "USD" },
  observable: "ATM_VOL",
  tenor: { unit: "YEARS", count: 1 },
};

describe("SeriesRegistry", () => {
  it("coalesces identical-argument cells onto one server series and refcounts", () => {
    const conn = new FakeSeriesConnection();
    const reg = new SeriesRegistry(conn);
    const a: SeriesTick[] = [];
    const b: SeriesTick[] = [];
    const subA = reg.acquire(REQ, (t) => a.push(t));
    const subB = reg.acquire(REQ, (t) => b.push(t));
    // One shared server series for two identical cells.
    expect(conn.opened).toHaveLength(1);
    expect(reg.liveSeriesCount()).toBe(1);
    expect(reg.totalRefcount()).toBe(2);

    // A snapshot seeds the baseline from the newest history point; a point updates.
    const opened0 = conn.opened[0];
    expect(opened0).toBeDefined();
    const id = opened0!.id;
    const subId = id;
    conn.emit({
      kind: "series_snapshot",
      snapshot: {
        subscriptionId: subId,
        sequence: 10n,
        pair: REQ.pair,
        observable: "ATM_VOL",
        points: [
          { subscriptionId: subId, sequence: 9n, value: 0.1, epochNanos: 1n },
          { subscriptionId: subId, sequence: 10n, value: 0.102, epochNanos: 2n },
        ],
        epochNanos: 2n,
      },
    });
    conn.emit({
      kind: "series_point",
      point: { subscriptionId: subId, sequence: 11n, value: 0.103, epochNanos: 3n },
    });

    // Both cells saw the seeded baseline (0.102) then the live point (0.103).
    expect(a.at(-1)?.value).toBeCloseTo(0.103, 12);
    expect(b.at(-1)?.value).toBeCloseTo(0.103, 12);
    expect(a.at(-1)?.baselined).toBe(true);

    // Releasing one keeps the server series; releasing the last tears it down.
    subA.release();
    expect(conn.closed).toHaveLength(0);
    expect(reg.totalRefcount()).toBe(1);
    subB.release();
    expect(conn.closed).toEqual([id]);
    expect(reg.liveSeriesCount()).toBe(0);
    reg.dispose();
  });

  it("keys distinct observables / wings to distinct server series", () => {
    const conn = new FakeSeriesConnection();
    const reg = new SeriesRegistry(conn);
    reg.acquire(REQ, () => {});
    reg.acquire({ ...REQ, observable: "RISK_REVERSAL", delta: 0.25 }, () => {});
    reg.acquire({ ...REQ, observable: "RISK_REVERSAL", delta: 0.1 }, () => {});
    expect(conn.opened).toHaveLength(3);
    expect(reg.liveSeriesCount()).toBe(3);
    reg.dispose();
  });

  it("seriesKey is stable and discriminates the keying fields", () => {
    expect(seriesKey(REQ)).toBe(seriesKey({ ...REQ }));
    expect(seriesKey(REQ)).not.toBe(seriesKey({ ...REQ, observable: "SPOT" }));
    expect(seriesKey({ ...REQ, observable: "RISK_REVERSAL", delta: 0.25 })).not.toBe(
      seriesKey({ ...REQ, observable: "RISK_REVERSAL", delta: 0.1 }),
    );
  });
});
