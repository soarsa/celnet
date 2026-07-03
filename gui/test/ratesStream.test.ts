/**
 * Fixed-income LIVE STREAMING — the offline (`?mock`) rates line, driven through
 * the REAL `MockTransport` StreamSession seam (no server, no mocks-of-our-own).
 *
 * The faithfulness contract the server guarantees (crates/celnet-server/src
 * /services/stream.rs): the baseline `RatesStreamSnapshot` at curve-shift 0 equals
 * `price_rates(instrument, curve_set)` EXACTLY, and each `RatesStreamUpdate`
 * re-prices the baseline curve shifted by `curveShift` through the SAME pricer.
 * These tests assert the OFFLINE line reproduces that against the in-browser
 * `priceRatesInstrumentOffline` mirror the rates unary edge already uses — so the
 * `?mock` demo is honest, not a fabricated tape.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MockTransport } from "../src/data/mockSource";
import {
  DEFAULT_USD_SOFR_CURVE,
  priceRatesInstrumentOffline,
} from "../src/data/ratesPricing";
import type { RatesCurveSet, RatesInstrument } from "../src/data/contract";
import type { StreamEvent } from "../src/data/transport";

const curve: RatesCurveSet = DEFAULT_USD_SOFR_CURVE;

/** A parallel-shifted curve copy — the test-side mirror of the mock's shift. */
function shifted(c: RatesCurveSet, shift: number): RatesCurveSet {
  return { ...c, pillars: c.pillars.map((p) => ({ ...p, parRate: p.parRate + shift })) };
}

const ARMS: { label: string; instrument: RatesInstrument }[] = [
  {
    label: "OIS 5Y",
    instrument: {
      kind: "ois",
      ois: { tenorYears: 5, fixedRate: 0.0405, notional: 50_000_000, direction: "PAY_FIXED" },
    },
  },
  {
    label: "IRS 5Y",
    instrument: {
      kind: "irs",
      irs: {
        tenorYears: 5,
        fixedRate: 0.0405,
        notional: 50_000_000,
        direction: "PAY_FIXED",
        fixedFrequency: "SEMI_ANNUAL",
        fixedDayCount: "ACT_360",
        floatFrequency: "QUARTERLY",
        floatDayCount: "ACT_360",
      },
    },
  },
  {
    label: "FRA 3x6",
    instrument: {
      kind: "fra",
      fra: {
        startMonths: 3,
        endMonths: 6,
        fixedRate: 0.043,
        notional: 50_000_000,
        direction: "PAY_FIXED",
        accrualBasis: "ACT_360",
      },
    },
  },
  {
    label: "Bond 10Y",
    instrument: {
      kind: "bond",
      bond: {
        couponRate: 0.04,
        couponFrequency: "SEMI_ANNUAL",
        dayCount: "THIRTY_360_BOND_BASIS",
        maturityDate: {
          year: curve.referenceDate.year + 10,
          month: curve.referenceDate.month,
          day: curve.referenceDate.day,
        },
        redemption: 100,
        position: "LONG",
      },
    },
  },
];

describe("rates stream (offline) — baseline == price_rates", () => {
  it.each(ARMS)("the $label baseline snapshot equals priceRatesInstrumentOffline exactly", ({ instrument }) => {
    const transport = new MockTransport();
    const session = transport.openStreamSession();
    const events: StreamEvent[] = [];
    session.onEvent((e) => events.push(e));

    // The baseline snapshot is emitted synchronously inside subscribeRates().
    session.subscribeRates(instrument, curve, "line");

    const baseline = events.find((e) => e.kind === "ratesSnapshot");
    expect(baseline).toBeDefined();
    if (baseline?.kind !== "ratesSnapshot") throw new Error("no baseline");
    expect(baseline.snapshot.sequence).toBe(1n);
    expect(baseline.snapshot.curveShift).toBe(0);
    // The whole priced result is byte-for-byte the offline `price_rates`.
    expect(baseline.snapshot.result).toEqual(priceRatesInstrumentOffline(curve, instrument));

    session.close();
  });
});

describe("rates stream (offline) — ticks are conflated reprices at the shifted curve", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("each RatesStreamUpdate re-prices the baseline curve shifted by its curveShift", () => {
    const instrument = ARMS[0]!.instrument;
    const transport = new MockTransport({ tickMs: 100 });
    const session = transport.openStreamSession();
    const events: StreamEvent[] = [];
    session.onEvent((e) => events.push(e));

    session.subscribeRates(instrument, curve, "OIS 5Y");
    // Advance several ticks; the mock emits ONE conflated update per tick.
    vi.advanceTimersByTime(500);

    const updates = events.filter((e) => e.kind === "ratesUpdate");
    expect(updates.length).toBeGreaterThanOrEqual(1);
    for (const e of updates) {
      if (e.kind !== "ratesUpdate") continue;
      // The shift is a real ±1bp parallel move (never a fabricated value).
      expect(Math.abs(e.update.curveShift)).toBeLessThanOrEqual(0.0001);
      // The result is exactly the offline reprice at that shifted curve.
      expect(e.update.result).toEqual(
        priceRatesInstrumentOffline(shifted(curve, e.update.curveShift), instrument),
      );
    }

    session.close();
  });

  it("streams a monotone sequence (baseline 1, then increasing updates) and multiplexes lines", () => {
    const transport = new MockTransport({ tickMs: 100 });
    const session = transport.openStreamSession();
    const events: StreamEvent[] = [];
    session.onEvent((e) => events.push(e));

    const idA = session.subscribeRates(ARMS[0]!.instrument, curve, "A");
    const idB = session.subscribeRates(ARMS[1]!.instrument, curve, "B");
    // Two lines get DISTINCT ids in the one shared SubscriptionId space.
    expect(idA).not.toBe(idB);

    vi.advanceTimersByTime(300);

    // Per line: a baseline at sequence 1 then updates with strictly increasing seq.
    for (const id of [idA, idB]) {
      const seqs = events
        .filter(
          (e) =>
            (e.kind === "ratesSnapshot" && e.snapshot.subscriptionId === id) ||
            (e.kind === "ratesUpdate" && e.update.subscriptionId === id),
        )
        .map((e) => (e.kind === "ratesSnapshot" ? e.snapshot.sequence : e.kind === "ratesUpdate" ? e.update.sequence : 0n));
      expect(seqs[0]).toBe(1n);
      for (let i = 1; i < seqs.length; i += 1) {
        expect(seqs[i]! > seqs[i - 1]!).toBe(true);
      }
    }
    // No click-to-trade token ever rides a rates line (no executed/reject either).
    expect(events.some((e) => e.kind === "executed" || e.kind === "reject")).toBe(false);

    session.close();
  });
});
