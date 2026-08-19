/**
 * A Hedge flow total and the ledger rows behind it must agree.
 *
 * The three totals (`CROSSED INTERNALLY` / `HEDGED EXTERNALLY` / `WAREHOUSED`) are sums
 * of the ledger's `internalCrossed` / `externalHedged` / `residual` columns. Selecting a
 * total drills the ledger down to the rows that produced it, so the predicate driving
 * that filter and the reducer computing the total have to stay in lockstep.
 *
 * This is the exact class of bug seen on UAT 2026-08-18: `WAREHOUSED 0` displayed above
 * thirteen rows carrying residual, because the total's inputs were filtered on one rule
 * and the rows on another. A total no row can explain — or rows no total counts — is a
 * trader being lied to about where their risk went.
 */
import { describe, expect, it } from "vitest";

import type { HedgeProvenance } from "../src/data/contract";
import { flowTotals } from "../src/lib/hedgeBuckets";
import { contributesTo, type HedgeLeg } from "../src/workspaces/HedgeDealsView";

/** A live (non-advisory) provenance row carrying the three flow amounts. */
function row(
  hedgeId: string,
  internalCrossed: number,
  externalHedged: number,
  residual: number,
  advisory = false,
): HedgeProvenance {
  return {
    hedgeId,
    book: "rates-usd",
    instrument: "BOND",
    firedAt: 1,
    metric: "dv01",
    threshold: 5000,
    netRisk: -4200,
    utilization: 0.84,
    band: "amber",
    policyPath: [0, 2],
    action: null,
    internalCrossed,
    externalHedged,
    residual,
    hedgePrice: 99.5,
    midAtFire: 99.5,
    slippageBp: 0,
    lpWon: null,
    advisory,
    lps: [],
    vehiclePlan: null,
  } as unknown as HedgeProvenance;
}

const FIELD = {
  crossed: "internalCrossed",
  hedged: "externalHedged",
  warehoused: "residual",
} as const;

const LEGS: HedgeLeg[] = ["crossed", "hedged", "warehoused"];

describe("hedge flow drill-down", () => {
  it("each total equals the sum of the rows its filter selects", () => {
    const rows = [
      row("h1", 100, 0, 0), // crossed only
      row("h2", 0, 250, 0), // shed only
      row("h3", 0, 0, 40), // a pure miss — all residual
      row("h4", 10, 60, 30), // a partial fill touches all three
      row("h5", 0, 0, 0), // an empty fire contributes nowhere
    ];
    const totals = flowTotals(rows);

    for (const leg of LEGS) {
      const summed = rows
        .filter((p) => contributesTo(p, leg))
        .reduce((acc, p) => acc + p[FIELD[leg]], 0);
      expect(summed).toBe(totals[leg]);
    }
  });

  it("a non-zero total always has at least one row to explain it", () => {
    const rows = [row("h1", 0, 0, 40), row("h2", 0, 0, 2000)];
    expect(flowTotals(rows).warehoused).toBeGreaterThan(0);
    // The UAT failure in one line: a total with no rows behind it.
    expect(rows.filter((p) => contributesTo(p, "warehoused")).length).toBeGreaterThan(0);
  });

  it("filters select only contributing rows, never a zero-amount row", () => {
    const rows = [row("h1", 0, 0, 0), row("h2", 5, 0, 0)];
    expect(rows.filter((p) => contributesTo(p, "crossed")).map((p) => p.hedgeId)).toEqual(["h2"]);
    expect(rows.filter((p) => contributesTo(p, "hedged"))).toHaveLength(0);
    expect(rows.filter((p) => contributesTo(p, "warehoused"))).toHaveLength(0);
  });

  it("advisory rows are excluded from the totals, so a drill-down cannot resurrect them", () => {
    // `flowTotals` drops advisory (dry-run) records. A row that never traded must not
    // appear behind a total either — the two must agree on what counts as live.
    const rows = [row("live", 0, 0, 100), row("dry", 0, 0, 999, true)];
    const total = flowTotals(rows).warehoused;
    expect(total).toBe(100);
    const summed = rows
      .filter((p) => !p.advisory && contributesTo(p, "warehoused"))
      .reduce((acc, p) => acc + p.residual, 0);
    expect(summed).toBe(total);
  });
});
