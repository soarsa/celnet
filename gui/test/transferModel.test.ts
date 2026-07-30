/**
 * transferModel unit tests — the pure client-side model behind the transfer ticket:
 * deterministic position synthesis (lots sum to the book), effective-desk resolution
 * up the tree, kind inference, and the before/after preview math (moved notional /
 * risk / realised P&L).
 */
import { describe, expect, it } from "vitest";

import type { RiskBook, RiskBookRisk } from "../src/data/contract";
import {
  aggregateLines,
  computePreview,
  effectiveDeskId,
  inferKind,
  movedNotionalOf,
  PAR_MARK,
  synthesizePositions,
} from "../src/workspaces/risktransfer/transferModel";

const row = (over: Partial<RiskBookRisk>): RiskBookRisk => ({
  bookId: "fi-rates-emea",
  name: "EMEA Rates",
  netNotional: 300_000_000,
  grossNotional: 300_000_000,
  positionCount: 3,
  delta: 1200,
  gamma: 40,
  vega: 800,
  theta: -60,
  dv01: 25_000,
  pnl: null,
  limits: [],
  ...over,
});

const books: RiskBook[] = [
  { id: "fx-emea", name: "FX EMEA", parentId: null, deskId: "emea", description: "", limits: null, enabled: true },
  { id: "fx-emea-vanilla", name: "Vanilla", parentId: "fx-emea", deskId: null, description: "", limits: null, enabled: true },
  { id: "fx-apac", name: "FX APAC", parentId: null, deskId: "apac", description: "", limits: null, enabled: true },
];

describe("synthesizePositions", () => {
  it("splits a row into lots whose notional + risk SUM to the row totals", () => {
    const r = row({ positionCount: 4 });
    const lots = synthesizePositions(r);
    expect(lots).toHaveLength(4);
    const agg = aggregateLines(lots);
    expect(agg.notionalBase).toBeCloseTo(r.netNotional, 3);
    expect(agg.risk.dv01).toBeCloseTo(r.dv01 ?? 0, 3);
    expect(agg.risk.delta).toBeCloseTo(r.delta, 3);
    expect(agg.risk.vega).toBeCloseTo(r.vega, 3);
  });

  it("is deterministic (stable ids + notionals across calls) and caps the lot count", () => {
    const a = synthesizePositions(row({ positionCount: 99 }));
    const b = synthesizePositions(row({ positionCount: 99 }));
    expect(a).toHaveLength(4); // capped
    expect(a.map((l) => l.id.toString())).toEqual(b.map((l) => l.id.toString()));
    expect(a.every((l) => l.id > 0n)).toBe(true);
  });

  it("handles a dv01-absent (FX) row by treating dv01 as 0", () => {
    const lots = synthesizePositions(row({ dv01: null, positionCount: 2 }));
    expect(aggregateLines(lots).risk.dv01).toBe(0);
  });
});

describe("effectiveDeskId", () => {
  it("returns a book's own desk", () => {
    expect(effectiveDeskId("fx-emea", books)).toBe("emea");
  });
  it("inherits the ancestor's desk when a sub-book has none", () => {
    expect(effectiveDeskId("fx-emea-vanilla", books)).toBe("emea");
  });
  it("returns empty when no ancestor carries a desk", () => {
    const orphan: RiskBook[] = [
      { id: "x", name: "X", parentId: null, deskId: null, description: "", limits: null, enabled: true },
    ];
    expect(effectiveDeskId("x", orphan)).toBe("");
  });
});

describe("inferKind", () => {
  it("same desk ⇒ RE_ATTRIBUTE", () => {
    expect(inferKind("emea", "emea", "")).toBe("RE_ATTRIBUTE");
  });
  it("different desk ⇒ DESK_TO_DESK", () => {
    expect(inferKind("emea", "marex", "")).toBe("DESK_TO_DESK");
  });
  it("a named trader ⇒ TRADER_TO_TRADER regardless of desk", () => {
    expect(inferKind("emea", "emea", "fi.trader@celnet.com")).toBe("TRADER_TO_TRADER");
    expect(inferKind("emea", "marex", "fi.trader@celnet.com")).toBe("TRADER_TO_TRADER");
  });
});

describe("movedNotionalOf", () => {
  it("Full moves the whole book net", () => {
    expect(movedNotionalOf(300, true, null)).toBe(300);
    expect(movedNotionalOf(-300, true, null)).toBe(-300);
  });
  it("Partial moves the requested magnitude in the net direction, bounded", () => {
    expect(movedNotionalOf(300, false, 100)).toBe(100);
    expect(movedNotionalOf(-300, false, 100)).toBe(-100); // short book → short slice
    expect(movedNotionalOf(300, false, 999)).toBe(300); // clamped to the net
  });
});

describe("computePreview", () => {
  const lots = synthesizePositions(row({ netNotional: 300_000_000, positionCount: 3 }));

  it("Full at the mark moves the whole selection with zero realised P&L", () => {
    const p = computePreview({
      selected: lots,
      quantityFull: true,
      partialNotional: null,
      agreedPrice: null,
      isAgreed: false,
      sourceNet: 300_000_000,
      targetNet: 50_000_000,
    });
    expect(p.movedNotional).toBeCloseTo(300_000_000, 2);
    expect(p.transferPrice).toBe(PAR_MARK);
    expect(p.realizedPnlSource).toBe(0);
    expect(p.sourceNetAfter).toBeCloseTo(0, 2);
    expect(p.targetNetAfter).toBeCloseTo(350_000_000, 2);
  });

  it("an AGREED off-mark price realises non-zero P&L in the source", () => {
    const p = computePreview({
      selected: lots,
      quantityFull: true,
      partialNotional: null,
      agreedPrice: 101,
      isAgreed: true,
      sourceNet: 300_000_000,
      targetNet: 0,
    });
    expect(p.transferPrice).toBe(101);
    // (101 - 100) * movedNotional / 100
    expect(p.realizedPnlSource).toBeCloseTo((300_000_000 * 1) / 100, 2);
  });

  it("Partial moves only the requested slice", () => {
    const p = computePreview({
      selected: lots,
      quantityFull: false,
      partialNotional: 90_000_000,
      agreedPrice: null,
      isAgreed: false,
      sourceNet: 300_000_000,
      targetNet: 0,
    });
    expect(p.movedNotional).toBeCloseTo(90_000_000, 2);
    expect(p.movedRisk.dv01).toBeCloseTo((25_000 * 90_000_000) / 300_000_000, 2);
  });
});
