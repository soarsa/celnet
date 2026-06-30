import { describe, expect, it } from "vitest";

import type {
  AggregateRatesRiskRequest,
  OisInstrument,
  RatesCurveSet,
  RatesPosition,
} from "../src/data/contract";
import { createMockTransport } from "../src/data/mockSource";
import { DEFAULT_USD_SOFR_CURVE, RatesPricingError } from "../src/data/ratesPricing";
import { DEFAULT_CONVENTIONS } from "../src/data/seed";

/**
 * Structural-identity tests for the in-browser rates portfolio-risk rollup
 * (`MockTransport.aggregateRatesRisk`). We assert the additive-aggregation
 * invariants the live `RiskService.AggregateRatesRisk` edge must satisfy —
 * netting is linear in the position set, the optional `(entity, book, ccy)`
 * scope narrows the contributing positions, and a node's key-rate ladder
 * reconciles to its parallel DV01 — rather than pinning opaque magic numbers.
 * This is the same discipline the offline OIS pricer (`ratesPricing.test.ts`)
 * and the Rust `celnet-server` rates-risk rollup are held to.
 */

const CURVE: RatesCurveSet = DEFAULT_USD_SOFR_CURVE;
const NOTIONAL = 10_000_000;

function ois(overrides: Partial<OisInstrument> = {}): OisInstrument {
  return {
    tenorYears: 5,
    fixedRate: 0.04,
    notional: NOTIONAL,
    direction: "RECEIVE_FIXED",
    ...overrides,
  };
}

function position(overrides: Partial<RatesPosition> = {}): RatesPosition {
  return {
    positionId: 1n,
    entity: 1,
    book: 100,
    instrument: ois(),
    ...overrides,
  };
}

function request(
  positions: readonly RatesPosition[],
  scope?: AggregateRatesRiskRequest["scope"],
): AggregateRatesRiskRequest {
  const req: AggregateRatesRiskRequest = { curveSet: CURVE, positions };
  if (scope !== undefined) req.scope = scope;
  return req;
}

describe("aggregateRatesRisk — additive rollup identities", () => {
  it("scales each node measure linearly with N identical positions", async () => {
    const t = createMockTransport();
    const one = await t.aggregateRatesRisk(request([position()]), DEFAULT_CONVENTIONS);
    const five = await t.aggregateRatesRisk(
      request([1, 2, 3, 4, 5].map((id) => position({ positionId: BigInt(id) }))),
      DEFAULT_CONVENTIONS,
    );

    expect(one.nodes).toHaveLength(1);
    expect(five.nodes).toHaveLength(1);
    const a = one.nodes[0]!;
    const b = five.nodes[0]!;
    expect(b.ccy).toBe(a.ccy);
    expect(b.netPv).toBeCloseTo(5 * a.netPv, 6);
    expect(b.netPv01).toBeCloseTo(5 * a.netPv01, 6);
    expect(b.netDv01).toBeCloseTo(5 * a.netDv01, 6);

    // The ladder scales bucket-for-bucket and stays tenor-aligned + ascending.
    expect(b.keyRateLadder).toHaveLength(a.keyRateLadder.length);
    b.keyRateLadder.forEach((bucket, i) => {
      expect(bucket.tenorYears).toBe(a.keyRateLadder[i]!.tenorYears);
      expect(bucket.dv01).toBeCloseTo(5 * a.keyRateLadder[i]!.dv01, 6);
    });
    const tenors = b.keyRateLadder.map((k) => k.tenorYears);
    expect(tenors).toEqual([...tenors].sort((x, y) => x - y));
  });

  it("nets payer against receiver to ~zero across every measure", async () => {
    const t = createMockTransport();
    const out = await t.aggregateRatesRisk(
      request([
        position({ positionId: 1n, instrument: ois({ direction: "RECEIVE_FIXED" }) }),
        position({ positionId: 2n, instrument: ois({ direction: "PAY_FIXED" }) }),
      ]),
      DEFAULT_CONVENTIONS,
    );
    expect(out.nodes).toHaveLength(1);
    const node = out.nodes[0]!;
    // A receiver and an identical payer are exact mirrors, so the additive book
    // nets to ~zero PV / PV01 / DV01 and a flat ladder.
    expect(Math.abs(node.netPv)).toBeLessThan(1e-2);
    expect(Math.abs(node.netPv01)).toBeLessThan(1e-6);
    expect(Math.abs(node.netDv01)).toBeLessThan(1e-6);
    node.keyRateLadder.forEach((bucket) => expect(Math.abs(bucket.dv01)).toBeLessThan(1e-6));
  });

  it("rejects an unsupported (non-USD) curve currency, as the server does", async () => {
    const t = createMockTransport();
    const eurCurve: RatesCurveSet = { ...CURVE, currency: "EUR" };
    await expect(
      t.aggregateRatesRisk({ curveSet: eurCurve, positions: [position()] }, DEFAULT_CONVENTIONS),
    ).rejects.toBeInstanceOf(RatesPricingError);
  });

  it("narrows the contributing positions by the book scope filter", async () => {
    const t = createMockTransport();
    const positions = [
      position({ positionId: 1n, book: 100 }),
      position({ positionId: 2n, book: 100 }),
      position({ positionId: 3n, book: 200 }),
    ];
    const all = await t.aggregateRatesRisk(request(positions), DEFAULT_CONVENTIONS);
    const book100 = await t.aggregateRatesRisk(
      request(positions, { book: 100 }),
      DEFAULT_CONVENTIONS,
    );

    // Book 100 holds 2 of 3 identical positions, so its netted DV01 is exactly
    // two-thirds of the unscoped three-position book (purely additive netting).
    expect(book100.nodes[0]!.netDv01).toBeCloseTo((2 / 3) * all.nodes[0]!.netDv01, 6);
    expect(book100.nodes[0]!.netPv).toBeCloseTo((2 / 3) * all.nodes[0]!.netPv, 6);
  });

  it("narrows the contributing positions by the entity scope filter", async () => {
    const t = createMockTransport();
    const positions = [
      position({ positionId: 1n, entity: 1 }),
      position({ positionId: 2n, entity: 2 }),
    ];
    const entity1 = await t.aggregateRatesRisk(
      request(positions, { entity: 1 }),
      DEFAULT_CONVENTIONS,
    );
    const single = await t.aggregateRatesRisk(
      request([position({ positionId: 9n, entity: 1 })]),
      DEFAULT_CONVENTIONS,
    );
    // Scoping to entity 1 keeps exactly one of the two positions, matching a
    // single-position book of the same instrument.
    expect(entity1.nodes[0]!.netDv01).toBeCloseTo(single.nodes[0]!.netDv01, 6);
  });

  it("yields an empty rollup when the scope excludes every position", async () => {
    const t = createMockTransport();
    const out = await t.aggregateRatesRisk(
      request([position({ book: 100 })], { book: 999 }),
      DEFAULT_CONVENTIONS,
    );
    expect(out.nodes).toHaveLength(0);
  });

  it("reconciles a node's key-rate ladder to its net DV01 on a tight tolerance", async () => {
    const t = createMockTransport();
    const out = await t.aggregateRatesRisk(
      request([
        position({ positionId: 1n, instrument: ois({ tenorYears: 5, fixedRate: 0.045 }) }),
        position({ positionId: 2n, instrument: ois({ tenorYears: 2, fixedRate: 0.038 }) }),
      ]),
      DEFAULT_CONVENTIONS,
    );
    const node = out.nodes[0]!;
    // The sum of the independent per-pillar 1bp bumps equals the single parallel
    // 1bp bump to first order (same identity as ratesPricing.test.ts), preserved
    // under additive aggregation — reconcile on a tight relative tolerance.
    const ladderSum = node.keyRateLadder.reduce((acc, k) => acc + k.dv01, 0);
    expect(Math.abs(ladderSum - node.netDv01) / Math.abs(node.netDv01)).toBeLessThan(1e-3);
  });
});
