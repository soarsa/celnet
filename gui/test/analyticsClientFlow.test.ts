/**
 * Client-flow analytics — the codec round-trip, the FlowGroupBy enum projection,
 * the offline mock data shape, and the capability/domain gating of the Analytics
 * tab. Pure (no React); the workspace render/format lives in
 * analyticsClientFlowWorkspace.test.tsx.
 */
import { describe, expect, it } from "vitest";

import {
  clientFlowMetricsFromWire,
  listClientFlowMetricsRequestToWire,
  listClientFlowMetricsResponseFromWire,
} from "../src/data/wsCodec";
import { flowGroupBy } from "../src/data/enums";
import type { FlowGroupBy } from "../src/data/contract";
import { CAPABILITY_ACTIONS } from "../src/data/contract";
import { MockTransport } from "../src/data/mockSource";
import {
  ANALYTICS_WORKSPACES,
  DOMAINS,
  domainAccessible,
  workspaceAccessible,
  type NavAuth,
} from "../src/lib/commands";
import { COMPONENT_ACCESS, TRADER_HELD_BACK_ACTIONS } from "../src/lib/capabilityMatrix";
import type { WireObject } from "../src/data/wsCodec";

// --- FlowGroupBy enum codec (proto tags 0..3, zero-based) -------------------

describe("flowGroupBy enum codec", () => {
  it("maps each member to its proto tag (CLIENT=0 … ASSET=3) and back", () => {
    const members: FlowGroupBy[] = ["client", "counterparty", "instrument", "asset"];
    members.forEach((m, i) => {
      expect(flowGroupBy.toWire(m)).toBe(i);
      expect(flowGroupBy.fromWire(i)).toBe(m);
    });
  });

  it("decodes an unknown tag to the proto3 zero member (client)", () => {
    expect(flowGroupBy.fromWire(99)).toBe("client");
  });
});

// --- request framing --------------------------------------------------------

describe("listClientFlowMetricsRequestToWire", () => {
  it("frames the group_by tag and omits an absent window", () => {
    expect(listClientFlowMetricsRequestToWire("asset")).toEqual({ group_by: 3 });
    expect(listClientFlowMetricsRequestToWire("client")).toEqual({ group_by: 0 });
  });

  it("includes both epoch-nanos bounds when the window is present", () => {
    const wire = listClientFlowMetricsRequestToWire("counterparty", {
      fromNanos: 1_000n,
      toNanos: 2_000n,
    });
    expect(wire).toEqual({ group_by: 1, from_nanos: 1000, to_nanos: 2000 });
  });
});

// --- response decode (optional doubles → number | undefined) ----------------

/** A fully-populated wire row (every optional present). */
function fullWireRow(): WireObject {
  return {
    label: "Millennium Capital",
    quote_count: 120,
    traded_count: 84,
    traded_notional: 640_000_000,
    gross_pnl: 82_000,
    total_markout: 9_000,
    total_hedge_cost: 6_500,
    net_pnl: 66_500,
    dpm_gross: 128.125,
    dpm_net: 103.9,
    captured_vs_offered: 0.73,
    mean_cover_distance: 0.6,
    breakeven_spread: 24.2,
    quote_to_trade_ratio: 1.43,
    hit_rate: 0.7,
    fishing_score: 0.0,
  };
}

describe("clientFlowMetricsFromWire", () => {
  it("decodes every field of a fully-populated row", () => {
    const r = clientFlowMetricsFromWire(fullWireRow());
    expect(r.label).toBe("Millennium Capital");
    expect(r.quoteCount).toBe(120);
    expect(r.tradedCount).toBe(84);
    expect(r.tradedNotional).toBe(640_000_000);
    expect(r.grossPnl).toBe(82_000);
    expect(r.totalMarkout).toBe(9_000);
    expect(r.totalHedgeCost).toBe(6_500);
    expect(r.netPnl).toBe(66_500);
    expect(r.dpmGross).toBeCloseTo(128.125);
    expect(r.dpmNet).toBeCloseTo(103.9);
    expect(r.capturedVsOffered).toBeCloseTo(0.73);
    expect(r.meanCoverDistance).toBeCloseTo(0.6);
    expect(r.breakevenSpread).toBeCloseTo(24.2);
    expect(r.quoteToTradeRatio).toBeCloseTo(1.43);
    expect(r.hitRate).toBeCloseTo(0.7);
    expect(r.fishingScore).toBe(0);
  });

  it("maps a null/absent optional double to undefined (never 0/NaN)", () => {
    // The zero-trade fisher: every trade-denominator ratio is absent on the wire.
    const fisher: WireObject = {
      label: "IMC",
      quote_count: 420,
      traded_count: 0,
      traded_notional: 0,
      gross_pnl: 0,
      total_markout: 0,
      total_hedge_cost: 0,
      net_pnl: 0,
      dpm_gross: null,
      dpm_net: null,
      captured_vs_offered: null,
      mean_cover_distance: null,
      breakeven_spread: null,
      quote_to_trade_ratio: null,
      hit_rate: 0.0,
      fishing_score: 1.0,
    };
    const r = clientFlowMetricsFromWire(fisher);
    expect(r.dpmGross).toBeUndefined();
    expect(r.dpmNet).toBeUndefined();
    expect(r.capturedVsOffered).toBeUndefined();
    expect(r.meanCoverDistance).toBeUndefined();
    expect(r.breakevenSpread).toBeUndefined();
    expect(r.quoteToTradeRatio).toBeUndefined();
    expect(r.hitRate).toBe(0);
    expect(r.fishingScore).toBe(1);
    // Never coerced to a number.
    expect(Number.isNaN(r.dpmNet ?? NaN)).toBe(true); // (undefined ?? NaN) === NaN
    expect(r.dpmNet).not.toBe(0);
  });

  it("round-trips a two-row response envelope", () => {
    const wire: WireObject = { metrics: [fullWireRow()], group_by: 3 };
    const rows = listClientFlowMetricsResponseFromWire(wire);
    expect(rows).toHaveLength(1);
    expect(rows[0]?.label).toBe("Millennium Capital");
    // An empty/absent metrics array decodes to [].
    expect(listClientFlowMetricsResponseFromWire({})).toEqual([]);
  });
});

// --- offline mock data shape ------------------------------------------------

describe("MockTransport.listClientFlowMetrics", () => {
  const t = new MockTransport();

  it("groups by client with at least one franchise and one fisher, spanning both assets", async () => {
    const rows = await t.listClientFlowMetrics("client");
    expect(rows.length).toBeGreaterThanOrEqual(6);

    // A franchise line: positive net $/mm, low fishing, defined ratios.
    const franchise = rows.find((r) => r.label === "Millennium Capital");
    expect(franchise).toBeDefined();
    expect(franchise?.dpmNet).toBeGreaterThan(0);
    expect(franchise?.netPnl).toBeGreaterThan(0);
    expect(franchise?.fishingScore).toBeLessThan(0.33);
    expect(franchise?.hitRate).toBeGreaterThan(0);

    // A pure fisher (zero fills): fishing ≈ 1, every trade-denominator ratio absent.
    const fisher = rows.find((r) => r.label === "IMC");
    expect(fisher).toBeDefined();
    expect(fisher?.fishingScore).toBeGreaterThan(0.66);
    expect(fisher?.tradedCount).toBe(0);
    expect(fisher?.dpmNet).toBeUndefined();
    expect(fisher?.quoteToTradeRatio).toBeUndefined();
    expect(fisher?.hitRate).toBe(0);
  });

  it("is key-ordered (labels ascending) and never emits NaN in any numeric field", async () => {
    for (const gb of ["client", "counterparty", "instrument", "asset"] as FlowGroupBy[]) {
      const rows = await t.listClientFlowMetrics(gb);
      const labels = rows.map((r) => r.label);
      expect(labels).toEqual([...labels].sort((a, b) => a.localeCompare(b)));
      for (const r of rows) {
        for (const v of Object.values(r) as unknown[]) {
          if (typeof v === "number") expect(Number.isNaN(v)).toBe(false);
        }
      }
    }
  });

  it("splits into exactly the two product rows under the asset grouping", async () => {
    const rows = await t.listClientFlowMetrics("asset");
    expect(rows.map((r) => r.label).sort()).toEqual(["FX Options", "Fixed Income"]);
    // Both products carry real traded flow.
    for (const r of rows) expect(r.tradedNotional).toBeGreaterThan(0);
  });

  it("folds a client active in both assets into one client row (Citadel)", async () => {
    const byClient = await t.listClientFlowMetrics("client");
    const citadel = byClient.filter((r) => r.label === "Citadel");
    expect(citadel).toHaveLength(1);
    // Its notional exceeds either single-asset leg — it summed both.
    expect(citadel[0]?.tradedNotional).toBeGreaterThan(430_000_000);
  });

  it("derives net $/mm exactly from the summed accumulators", async () => {
    const rows = await t.listClientFlowMetrics("client");
    for (const r of rows) {
      if (r.tradedNotional > 0) {
        const mm = r.tradedNotional / 1_000_000;
        expect(r.dpmNet).toBeCloseTo(r.netPnl / mm, 6);
        expect(r.dpmGross).toBeCloseTo(r.grossPnl / mm, 6);
      } else {
        expect(r.dpmNet).toBeUndefined();
      }
      expect(r.netPnl).toBeCloseTo(r.grossPnl - r.totalMarkout - r.totalHedgeCost, 6);
    }
  });
});

// --- capability model + tab gating ------------------------------------------

function authWith(caps: string[], isAdmin = false): NavAuth {
  const held = new Set(caps);
  return { isAdmin, can: (action, asset) => held.has(`${action}·${asset}`) };
}

describe("view_analytics capability model", () => {
  it("view_analytics is the 15th action, held back from the default trader bundle", () => {
    expect(CAPABILITY_ACTIONS).toContain("view_analytics");
    expect(CAPABILITY_ACTIONS).toHaveLength(15);
    expect(TRADER_HELD_BACK_ACTIONS.has("view_analytics")).toBe(true);
  });

  it("is surfaced as a cross-asset read-only component in the grant grid", () => {
    const analytics = COMPONENT_ACCESS.find((c) => c.id === "analytics");
    expect(analytics).toBeDefined();
    expect(analytics?.readActions).toEqual(["view_analytics"]);
    expect(analytics?.writeActions).toEqual([]);
    expect([...(analytics?.assets ?? [])].sort()).toEqual(["fixed_income", "fx_options"]);
  });
});

describe("Analytics tab gating", () => {
  it("DOMAINS carries an analytics tab next to Administration", () => {
    const ids = DOMAINS.map((d) => d.id);
    expect(ids).toContain("analytics");
    expect(ids.indexOf("analytics")).toBe(ids.indexOf("admin") - 1);
  });

  it("clientflow is a member of ANALYTICS_WORKSPACES", () => {
    expect(ANALYTICS_WORKSPACES.has("clientflow")).toBe(true);
  });

  it("the tab + workspace are HIDDEN for a signed-in user without view_analytics", () => {
    const trader = authWith(["view·fx_options", "view·fixed_income", "execute·fixed_income"]);
    expect(domainAccessible("analytics", trader)).toBe(false);
    expect(workspaceAccessible("clientflow", trader)).toBe(false);
  });

  it("holding view_analytics on EITHER asset admits the tab (cross-product OR)", () => {
    const fxOnly = authWith(["view·fx_options", "view_analytics·fx_options"]);
    expect(domainAccessible("analytics", fxOnly)).toBe(true);
    expect(workspaceAccessible("clientflow", fxOnly)).toBe(true);

    const fiOnly = authWith(["view·fixed_income", "view_analytics·fixed_income"]);
    expect(domainAccessible("analytics", fiOnly)).toBe(true);
    expect(workspaceAccessible("clientflow", fiOnly)).toBe(true);
  });

  it("an admin (grant-all in practice) reaches it", () => {
    const admin: NavAuth = { isAdmin: true, can: () => true };
    expect(domainAccessible("analytics", admin)).toBe(true);
    expect(workspaceAccessible("clientflow", admin)).toBe(true);
  });
});
