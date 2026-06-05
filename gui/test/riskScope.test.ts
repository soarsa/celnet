/**
 * riskView scope-reducer tests — the thin, server-deferring glue that maps the
 * toolbar Scope (Firm · Desk · Book · Pair) onto the contract's `RiskDimension`
 * group-by + limit scope, and assembles the USD `ReportingNumeraire` from live
 * watched-pair spots. NO aggregation happens here (the server owns that); these
 * pin the pure scope→dimension reducer and the cross-rate derivation.
 *
 * Exercises the REAL `src/data/riskView.ts` through its public surface.
 */
import { describe, expect, it } from "vitest";

import {
  BOOK_VEGA_PILLARS,
  REPORTING_CCY,
  dimensionForScope,
  limitScopeForScope,
  principalForScope,
  reportingNumeraire,
} from "../src/data/riskView";
import { FIRM_SCOPE_ROOT, type ScopeContext, type ScopeNode } from "../src/app/AppContext";
import type { PairContext } from "../src/data/seed";
import { PAIRS } from "../src/data/seed";

/** Build a grant-all scope at a given drill path. */
function scopeAt(...nodes: ScopeNode[]): ScopeContext {
  return { principal: "grant-all", path: [FIRM_SCOPE_ROOT, ...nodes], groupBy: "none" };
}

describe("dimensionForScope — group-by axis follows the drill tail", () => {
  it("groups by CCY_PAIR at the firm root", () => {
    expect(dimensionForScope(scopeAt())).toBe("CCY_PAIR");
  });

  it("groups by BOOK when scoped to a desk", () => {
    expect(dimensionForScope(scopeAt({ level: "desk", label: "EMEA" }))).toBe("BOOK");
  });

  it("groups by TRADER when scoped to a book", () => {
    expect(
      dimensionForScope(
        scopeAt({ level: "desk", label: "EMEA" }, { level: "book", label: "VOL-1" }),
      ),
    ).toBe("TRADER");
  });

  it("groups by CCY_PAIR at the pair leaf", () => {
    expect(
      dimensionForScope(
        scopeAt(
          { level: "desk", label: "EMEA" },
          { level: "book", label: "VOL-1" },
          { level: "pair", label: "EUR/USD" },
        ),
      ),
    ).toBe("CCY_PAIR");
  });
});

describe("limitScopeForScope — org node the limit tree keys on", () => {
  it("reads limits at the FIRM apex (value 0) at the root", () => {
    expect(limitScopeForScope(scopeAt())).toEqual({ dimension: "FIRM", value: 0n });
  });

  it("reads limits at DESK / BOOK as the trader drills", () => {
    expect(limitScopeForScope(scopeAt({ level: "desk", label: "EMEA" }))).toEqual({
      dimension: "DESK",
      value: 0n,
    });
    expect(
      limitScopeForScope(
        scopeAt({ level: "desk", label: "EMEA" }, { level: "book", label: "VOL-1" }),
      ),
    ).toEqual({ dimension: "BOOK", value: 0n });
  });

  it("falls back to FIRM for a pair scope (a pair is not an org limit node)", () => {
    expect(
      limitScopeForScope(scopeAt({ level: "pair", label: "EUR/USD" })),
    ).toEqual({ dimension: "FIRM", value: 0n });
  });
});

describe("principalForScope — grant-all omits the principal today", () => {
  it("returns undefined so the server applies its grant-all default", () => {
    expect(principalForScope(scopeAt())).toBeUndefined();
  });
});

describe("reportingNumeraire — USD cross-rate derivation from live spots", () => {
  it("derives base→USD directly from a USD-quote pair (EUR/USD)", () => {
    const pairs: PairContext[] = [
      { pair: { base: "EUR", quote: "USD" }, market: { spot: 1.08, vol: 0.07, rDom: 0, rFor: 0 }, pipDecimals: 4 },
    ];
    const n = reportingNumeraire(pairs);
    expect(n.numeraire).toBe(REPORTING_CCY);
    expect(n.rates).toEqual([{ ccy: "EUR", rate: 1.08 }]);
  });

  it("derives quote→USD as 1/spot from a USD-base pair (USD/JPY)", () => {
    const pairs: PairContext[] = [
      { pair: { base: "USD", quote: "JPY" }, market: { spot: 150, vol: 0.1, rDom: 0, rFor: 0 }, pipDecimals: 2 },
    ];
    const n = reportingNumeraire(pairs);
    expect(n.rates).toEqual([{ ccy: "JPY", rate: 1 / 150 }]);
  });

  it("bridges a cross pair (no USD leg) through a known USD rate", () => {
    // EUR→USD known from EUR/USD; EUR/GBP then yields GBP→USD = EUR_USD / spot.
    const pairs: PairContext[] = [
      { pair: { base: "EUR", quote: "USD" }, market: { spot: 1.08, vol: 0.07, rDom: 0, rFor: 0 }, pipDecimals: 4 },
      { pair: { base: "EUR", quote: "GBP" }, market: { spot: 0.85, vol: 0.07, rDom: 0, rFor: 0 }, pipDecimals: 4 },
    ];
    const n = reportingNumeraire(pairs);
    const byCcy = new Map(n.rates.map((r) => [r.ccy, r.rate]));
    expect(byCcy.get("EUR")).toBeCloseTo(1.08, 12);
    // GBP→USD: GBP per EUR is 0.85, so USD per GBP = 1.08 / 0.85.
    expect(byCcy.get("GBP")).toBeCloseTo(1.08 / 0.85, 12);
  });

  it("omits USD itself and a ccy with no path to USD (never invents a rate)", () => {
    const pairs: PairContext[] = [
      { pair: { base: "TRY", quote: "ZAR" }, market: { spot: 1.5, vol: 0.2, rDom: 0, rFor: 0 }, pipDecimals: 4 },
    ];
    const n = reportingNumeraire(pairs);
    // Neither leg touches USD and there is no bridge ⇒ no rates derived.
    expect(n.rates).toEqual([]);
    expect(n.rates.find((r) => r.ccy === REPORTING_CCY)).toBeUndefined();
  });

  it("returns rates sorted by currency code, on the real seeded universe", () => {
    const n = reportingNumeraire(PAIRS);
    const codes = n.rates.map((r) => r.ccy);
    expect([...codes].sort()).toEqual(codes);
    // Every seeded pair touches USD, so every non-USD leg resolves a rate.
    expect(codes).toContain("EUR");
    expect(codes).toContain("JPY");
  });
});

describe("BOOK_VEGA_PILLARS — the explicit, stable ladder grid", () => {
  it("is the 6 tenors × 5 delta pillars Cartesian grid", () => {
    expect(BOOK_VEGA_PILLARS).toHaveLength(30);
    // Tenors in DAYS, deltas in BASIS POINTS, per the contract RiskVegaPillar.
    for (const p of BOOK_VEGA_PILLARS) {
      expect(Number.isInteger(p.tenorDays)).toBe(true);
      expect(Number.isInteger(p.deltaBp)).toBe(true);
    }
    expect(BOOK_VEGA_PILLARS.some((p) => p.deltaBp === 5000)).toBe(true);
  });
});
