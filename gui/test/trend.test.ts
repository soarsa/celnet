/**
 * trend.ts tests — the tenor LABEL grammar (display-only ON/TN/SN → W/M/Y) and
 * the TrendMode catalogue (which series are live vs honestly-gated, their units,
 * and the Tenor ⇄ year-fraction maps a market-series subscription pins).
 *
 * Exercises the REAL `src/lib/trend.ts` through its public surface.
 */
import { describe, expect, it } from "vitest";

import {
  DEFAULT_TREND_MODE,
  TREND_MODES,
  TREND_WING_DELTA,
  tenorForYears,
  tenorLabel,
  tenorYearsOf,
  trendModeSpec,
  type TrendMode,
} from "../src/lib/trend";
import type { Tenor } from "../src/data/contract";

describe("tenorLabel — short-end + W/M/Y grammar", () => {
  it("labels the very short end ON/TN/SN", () => {
    expect(tenorLabel(1 / 365)).toBe("ON");
    expect(tenorLabel(2 / 365)).toBe("TN");
    expect(tenorLabel(3 / 365)).toBe("SN");
  });

  it("labels whole weeks below a month", () => {
    expect(tenorLabel(7 / 365)).toBe("1W");
    expect(tenorLabel(14 / 365)).toBe("2W");
    expect(tenorLabel(21 / 365)).toBe("3W");
  });

  it("labels whole months below a year, capped at 11M", () => {
    expect(tenorLabel(1 / 12)).toBe("1M");
    expect(tenorLabel(3 / 12)).toBe("3M");
    expect(tenorLabel(6 / 12)).toBe("6M");
    expect(tenorLabel(11 / 12)).toBe("11M");
  });

  it("labels years whole, else to one decimal", () => {
    expect(tenorLabel(1)).toBe("1Y");
    expect(tenorLabel(2)).toBe("2Y");
    expect(tenorLabel(1.5)).toBe("1.5Y");
  });

  it("returns the honest em-dash for non-positive / non-finite input", () => {
    expect(tenorLabel(0)).toBe("—");
    expect(tenorLabel(-1)).toBe("—");
    expect(tenorLabel(Number.NaN)).toBe("—");
    expect(tenorLabel(Number.POSITIVE_INFINITY)).toBe("—");
  });
});

describe("TREND_MODES — catalogue shape + availability", () => {
  it("marks PREMIUM and every market-observable mode live, VEGA/PNL gated", () => {
    const byId = new Map(TREND_MODES.map((m) => [m.id, m]));
    for (const id of ["PREMIUM", "ATM_VOL", "RR", "BF", "SPOT", "FORWARD"] as TrendMode[]) {
      expect(byId.get(id)?.available).toBe(true);
    }
    expect(byId.get("VEGA")?.available).toBe(false);
    expect(byId.get("PNL")?.available).toBe(false);
  });

  it("pins each market-backed mode to its observable; PREMIUM/gated have none", () => {
    const byId = new Map(TREND_MODES.map((m) => [m.id, m]));
    expect(byId.get("PREMIUM")?.observable).toBeNull();
    expect(byId.get("ATM_VOL")?.observable).toBe("ATM_VOL");
    expect(byId.get("RR")?.observable).toBe("RISK_REVERSAL");
    expect(byId.get("BF")?.observable).toBe("BUTTERFLY");
    expect(byId.get("SPOT")?.observable).toBe("SPOT");
    expect(byId.get("FORWARD")?.observable).toBe("FORWARD");
    expect(byId.get("VEGA")?.observable).toBeNull();
  });

  it("flags only RR/BF as needing a signed delta wing", () => {
    const byId = new Map(TREND_MODES.map((m) => [m.id, m]));
    expect(byId.get("RR")?.needsDelta).toBe(true);
    expect(byId.get("BF")?.needsDelta).toBe(true);
    expect(byId.get("ATM_VOL")?.needsDelta).toBeUndefined();
    expect(TREND_WING_DELTA).toBe(0.25);
  });

  it("trendModeSpec falls back to the default mode on an unknown id", () => {
    expect(trendModeSpec("PREMIUM").id).toBe("PREMIUM");
    expect(trendModeSpec("nonsense" as TrendMode).id).toBe(DEFAULT_TREND_MODE);
    expect(DEFAULT_TREND_MODE).toBe("PREMIUM");
  });
});

describe("tenorForYears ⇄ tenorYearsOf — pillar mapping", () => {
  it("maps the short end to OVERNIGHT", () => {
    expect(tenorForYears(1 / 365)).toEqual({ unit: "OVERNIGHT", count: 1 });
    expect(tenorForYears(0)).toEqual({ unit: "OVERNIGHT", count: 1 });
  });

  it("maps sub-month to whole weeks, sub-year to months, else years", () => {
    expect(tenorForYears(7 / 365)).toEqual({ unit: "WEEKS", count: 1 });
    expect(tenorForYears(3 / 12)).toEqual({ unit: "MONTHS", count: 3 });
    expect(tenorForYears(1)).toEqual({ unit: "YEARS", count: 1 });
    expect(tenorForYears(2)).toEqual({ unit: "YEARS", count: 2 });
  });

  it("round-trips a standard pillar tenor back to a close year fraction", () => {
    const tenors: Tenor[] = [
      { unit: "WEEKS", count: 2 },
      { unit: "MONTHS", count: 6 },
      { unit: "YEARS", count: 1 },
    ];
    for (const t of tenors) {
      const years = tenorYearsOf(t);
      expect(tenorForYears(years)).toEqual(t);
    }
  });

  it("maps the short-end units to their day band", () => {
    expect(tenorYearsOf({ unit: "OVERNIGHT", count: 1 })).toBeCloseTo(1 / 365, 12);
    expect(tenorYearsOf({ unit: "TOM_NEXT", count: 1 })).toBeCloseTo(2 / 365, 12);
    expect(tenorYearsOf({ unit: "SPOT_NEXT", count: 1 })).toBeCloseTo(3 / 365, 12);
    expect(tenorYearsOf({ unit: "MONTHS", count: 1 })).toBeCloseTo(1 / 12, 12);
  });
});
