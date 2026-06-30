import { describe, expect, it } from "vitest";

import { createMockTransport } from "../src/data/mockSource";
import { DEFAULT_CONVENTIONS } from "../src/data/seed";
import {
  buildRatesRiskRequest,
  buildScope,
  defaultRatesRiskRows,
  ladderMaxAbs,
  rowToPosition,
  type RatesRiskRow,
} from "../src/workspaces/RatesRiskWorkspace";

/**
 * Behavioural tests for the RatesRiskWorkspace's PURE helpers, exercised against
 * the in-app `MockTransport.aggregateRatesRisk` rollup (the same engine the live
 * `RiskService.AggregateRatesRisk` edge mirrors). We assert the request the
 * workspace builds from its editable rows is the correct wire shape, that the
 * rollup is additive and its key-rate ladder reconciles to the net DV01, and that
 * the `(entity, book, ccy)` scope narrows the contributing positions — the same
 * structural-identity discipline `ratesPricing.test.ts` / `aggregateRatesRisk.test.ts`
 * hold the offline core to. No React rendering is involved.
 */

const MM = 1_000_000;

function row(over: Partial<RatesRiskRow> = {}): RatesRiskRow {
  return {
    id: "r",
    entity: 1,
    book: 10,
    tenorYears: 5,
    fixedRatePct: 4.0,
    notionalMm: 100,
    direction: "RECEIVE_FIXED",
    ...over,
  };
}

describe("buildRatesRiskRequest — rows → wire positions", () => {
  it("projects each editable row onto its RatesPosition (pct→decimal, mm→notional)", () => {
    const r = row({
      entity: 7,
      book: 33,
      tenorYears: 10,
      fixedRatePct: 4.25,
      notionalMm: 50,
      direction: "PAY_FIXED",
    });
    const req = buildRatesRiskRequest([r]);
    expect(req.positions).toHaveLength(1);
    const p = req.positions[0]!;
    expect(p.entity).toBe(7);
    expect(p.book).toBe(33);
    expect(p.positionId).toBe(0n);
    expect(p.instrument.tenorYears).toBe(10);
    expect(p.instrument.fixedRate).toBeCloseTo(0.0425, 12);
    expect(p.instrument.notional).toBe(50 * MM);
    expect(p.instrument.direction).toBe("PAY_FIXED");
    // rowToPosition is the single projection used; index → informational positionId.
    expect(rowToPosition(r, 4).positionId).toBe(4n);
  });

  it("threads the curve set and omits scope/principal when absent", () => {
    const req = buildRatesRiskRequest([row(), row({ id: "r2" })]);
    expect(req.positions).toHaveLength(2);
    expect(req.curveSet.currency).toBe("USD");
    expect(req.scope).toBeUndefined();
    expect(req.principal).toBeUndefined();
  });

  it("seeds a populated, genuinely-curve-derived default book", () => {
    const rows = defaultRatesRiskRows();
    expect(rows.length).toBeGreaterThanOrEqual(3);
    // Every seed tenor is a real curve pillar tenor (derived, not arbitrary).
    const pillarTenors = new Set([1, 2, 3, 5, 7, 10, 15, 20, 30]);
    for (const r of rows) expect(pillarTenors.has(r.tenorYears)).toBe(true);
  });
});

describe("buildScope — the optional (entity, book, ccy) filter", () => {
  it("returns undefined when every field is blank (whole portfolio)", () => {
    expect(buildScope({ entity: "", book: "", ccy: "" })).toBeUndefined();
  });

  it("narrows on each present field and drops blank ones", () => {
    expect(buildScope({ entity: "2", book: "", ccy: "" })).toEqual({ entity: 2 });
    expect(buildScope({ entity: "", book: "30", ccy: "" })).toEqual({ book: 30 });
    expect(buildScope({ entity: "1", book: "10", ccy: "usd" })).toEqual({
      entity: 1,
      book: 10,
      ccy: "usd",
    });
  });
});

describe("aggregateRatesRisk via MockTransport — additive rollup + reconciliation", () => {
  it("nets additively across positions and reconciles the ladder to net DV01", async () => {
    const t = createMockTransport();
    // Both receivers, so the DV01s share sign and the net is comfortably non-zero.
    const a = row({ id: "a", tenorYears: 5, fixedRatePct: 4.2, notionalMm: 100 });
    const b = row({ id: "b", tenorYears: 10, fixedRatePct: 3.9, notionalMm: 60 });

    const ra = await t.aggregateRatesRisk(buildRatesRiskRequest([a]), DEFAULT_CONVENTIONS);
    const rb = await t.aggregateRatesRisk(buildRatesRiskRequest([b]), DEFAULT_CONVENTIONS);
    const rab = await t.aggregateRatesRisk(buildRatesRiskRequest([a, b]), DEFAULT_CONVENTIONS);

    // One settlement currency (USD) ⇒ exactly one node per rollup.
    expect(rab.nodes).toHaveLength(1);
    const na = ra.nodes[0]!;
    const nb = rb.nodes[0]!;
    const nab = rab.nodes[0]!;
    expect(nab.ccy).toBe("USD");

    // Additivity: the combined node is the sum of the singletons, measure by measure.
    expect(nab.netPv).toBeCloseTo(na.netPv + nb.netPv, 4);
    expect(nab.netPv01).toBeCloseTo(na.netPv01 + nb.netPv01, 4);
    expect(nab.netDv01).toBeCloseTo(na.netDv01 + nb.netDv01, 4);

    // The key-rate ladder buckets sum to the net DV01 to first order (<1e-3 rel) —
    // the same identity ratesPricing.test.ts holds the per-swap ladder to.
    const ladderSum = nab.keyRateLadder.reduce((s, k) => s + k.dv01, 0);
    expect(Math.abs(ladderSum - nab.netDv01) / Math.abs(nab.netDv01)).toBeLessThan(1e-3);

    // The ladder is per curve pillar, ascending, with a non-degenerate peak bucket.
    expect(nab.keyRateLadder.length).toBeGreaterThan(0);
    expect(ladderMaxAbs(nab)).toBeGreaterThan(0);
  });
});

describe("aggregateRatesRisk via MockTransport — scope narrows the rollup", () => {
  it("an entity filter rolls up only that entity's positions", async () => {
    const t = createMockTransport();
    const e1 = row({ id: "e1", entity: 1, tenorYears: 5, notionalMm: 100 });
    const e2 = row({ id: "e2", entity: 2, tenorYears: 7, notionalMm: 80 });

    const entityScope = buildScope({ entity: "1", book: "", ccy: "" });
    const scoped = await t.aggregateRatesRisk(
      buildRatesRiskRequest([e1, e2], { ...(entityScope ? { scope: entityScope } : {}) }),
      DEFAULT_CONVENTIONS,
    );
    const onlyE1 = await t.aggregateRatesRisk(
      buildRatesRiskRequest([e1]),
      DEFAULT_CONVENTIONS,
    );

    expect(scoped.nodes).toHaveLength(1);
    expect(scoped.nodes[0]!.netPv).toBeCloseTo(onlyE1.nodes[0]!.netPv, 6);
    expect(scoped.nodes[0]!.netDv01).toBeCloseTo(onlyE1.nodes[0]!.netDv01, 6);
  });

  it("a non-matching currency filter yields an empty rollup (no nodes)", async () => {
    const t = createMockTransport();
    const rows = [row({ id: "a" }), row({ id: "b", entity: 2 })];
    const eurScope = buildScope({ entity: "", book: "", ccy: "EUR" });
    const none = await t.aggregateRatesRisk(
      buildRatesRiskRequest(rows, { ...(eurScope ? { scope: eurScope } : {}) }),
      DEFAULT_CONVENTIONS,
    );
    expect(none.nodes).toHaveLength(0);
  });
});
