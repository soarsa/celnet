/**
 * Street-liquidity analytics — the codec round-trip, the request framing (window +
 * lp_id filter), the offline mock data shape, and the capability/domain gating of the
 * Street Liquidity workspace. Pure (no React); the workspace render/format lives in
 * analyticsStreetLiquidityWorkspace.test.tsx.
 */
import { describe, expect, it } from "vitest";

import {
  lpFlowMetricsFromWire,
  listLpFlowMetricsRequestToWire,
  listLpFlowMetricsResponseFromWire,
} from "../src/data/wsCodec";
import { MockTransport } from "../src/data/mockSource";
import {
  ANALYTICS_WORKSPACES,
  domainAccessible,
  workspaceAccessible,
  type NavAuth,
} from "../src/lib/commands";
import type { WireObject } from "../src/data/wsCodec";

// --- request framing --------------------------------------------------------

describe("listLpFlowMetricsRequestToWire", () => {
  it("frames an empty body when no window or lp filter is given", () => {
    expect(listLpFlowMetricsRequestToWire()).toEqual({});
  });

  it("includes both epoch-nanos bounds when the window is present", () => {
    expect(listLpFlowMetricsRequestToWire({ fromNanos: 1_000n, toNanos: 2_000n })).toEqual({
      from_nanos: 1000,
      to_nanos: 2000,
    });
  });

  it("includes the lp_id filter only when set + non-empty", () => {
    expect(listLpFlowMetricsRequestToWire(undefined, "XTX Markets")).toEqual({
      lp_id: "XTX Markets",
    });
    // Empty string ⇒ omitted (no filter).
    expect(listLpFlowMetricsRequestToWire(undefined, "")).toEqual({});
  });
});

// --- response decode (optional doubles → number | undefined) ----------------

/** A fully-populated wire row (both optional ratios present). */
function fullWireRow(): WireObject {
  return {
    lp_id: "XTX Markets",
    tick_count: 91_500,
    quote_count: 1_240,
    deals_won: 705,
    won_notional: 3_600_000_000,
    missed: 520,
    last_look_rejects: 12,
    win_rate: 0.5685,
    mean_cover: 0.9,
  };
}

describe("lpFlowMetricsFromWire", () => {
  it("decodes every field of a fully-populated row", () => {
    const r = lpFlowMetricsFromWire(fullWireRow());
    expect(r.lpId).toBe("XTX Markets");
    expect(r.tickCount).toBe(91_500);
    expect(r.quoteCount).toBe(1_240);
    expect(r.dealsWon).toBe(705);
    expect(r.wonNotional).toBe(3_600_000_000);
    expect(r.missed).toBe(520);
    expect(r.lastLookRejects).toBe(12);
    expect(r.winRate).toBeCloseTo(0.5685);
    expect(r.meanCover).toBeCloseTo(0.9);
  });

  it("maps a null/absent optional double to undefined (never 0/NaN)", () => {
    // The dormant LP: no quotes ⇒ win-rate absent; never cover ⇒ mean-cover absent.
    const dormant: WireObject = {
      lp_id: "Raiffeisen",
      tick_count: 800,
      quote_count: 0,
      deals_won: 0,
      won_notional: 0,
      missed: 0,
      last_look_rejects: 0,
      win_rate: null,
      mean_cover: null,
    };
    const r = lpFlowMetricsFromWire(dormant);
    expect(r.winRate).toBeUndefined();
    expect(r.meanCover).toBeUndefined();
    expect(r.winRate).not.toBe(0);
    expect(Number.isNaN(r.winRate ?? NaN)).toBe(true); // (undefined ?? NaN) === NaN
  });

  it("round-trips a response envelope (and an empty one decodes to [])", () => {
    const wire: WireObject = { metrics: [fullWireRow()] };
    const rows = listLpFlowMetricsResponseFromWire(wire);
    expect(rows).toHaveLength(1);
    expect(rows[0]?.lpId).toBe("XTX Markets");
    expect(listLpFlowMetricsResponseFromWire({})).toEqual([]);
  });
});

// --- offline mock data shape ------------------------------------------------

describe("MockTransport.listLpFlowMetrics", () => {
  const t = new MockTransport();

  it("returns a spread of LPs, lp_id-ordered, never emitting NaN", async () => {
    const rows = await t.listLpFlowMetrics();
    expect(rows.length).toBeGreaterThanOrEqual(8);
    const ids = rows.map((r) => r.lpId);
    expect(ids).toEqual([...ids].sort((a, b) => a.localeCompare(b)));
    for (const r of rows) {
      for (const v of Object.values(r) as unknown[]) {
        if (typeof v === "number") expect(Number.isNaN(v)).toBe(false);
      }
    }
  });

  it("has a tight winner (high win-rate, high deals), a chronic misser, and a last-look rejecter", async () => {
    const rows = await t.listLpFlowMetrics();

    // A tight winner: high win-rate, many deals, real won notional.
    const winner = rows.find((r) => r.lpId === "Citadel Securities");
    expect(winner).toBeDefined();
    expect(winner?.winRate ?? 0).toBeGreaterThan(0.4);
    expect(winner?.dealsWon ?? 0).toBeGreaterThan(200);
    expect(winner?.wonNotional ?? 0).toBeGreaterThan(0);

    // A chronic misser: quoted heavily, hardly wins (low win-rate, high missed).
    const misser = rows.find((r) => r.lpId === "DNB Markets");
    expect(misser).toBeDefined();
    expect(misser?.winRate ?? 1).toBeLessThan(0.1);
    expect(misser?.missed ?? 0).toBeGreaterThan(misser?.dealsWon ?? 0);

    // A last-look rejecter: a notable last-look-reject count.
    const rejecter = rows.find((r) => r.lpId === "Handelsbanken");
    expect(rejecter).toBeDefined();
    expect(rejecter?.lastLookRejects ?? 0).toBeGreaterThanOrEqual(50);
  });

  it("has a dormant LP whose win-rate and mean-cover are ABSENT (never 0)", async () => {
    const rows = await t.listLpFlowMetrics();
    const dormant = rows.find((r) => r.lpId === "Raiffeisen");
    expect(dormant).toBeDefined();
    expect(dormant?.quoteCount).toBe(0);
    expect(dormant?.winRate).toBeUndefined();
    expect(dormant?.meanCover).toBeUndefined();

    // A distinct never-cover LP: win-rate present, mean-cover absent (the two optionals are independent).
    const neverCover = rows.find((r) => r.lpId === "Swedbank");
    expect(neverCover?.winRate).toBeDefined();
    expect(neverCover?.meanCover).toBeUndefined();
  });

  it("derives win-rate exactly as deals won ÷ quotes", async () => {
    const rows = await t.listLpFlowMetrics();
    for (const r of rows) {
      if (r.quoteCount > 0) {
        expect(r.winRate).toBeCloseTo(r.dealsWon / r.quoteCount, 9);
      } else {
        expect(r.winRate).toBeUndefined();
      }
    }
  });

  it("narrows to a single LP when the lp_id filter is set", async () => {
    const rows = await t.listLpFlowMetrics(undefined, "XTX Markets");
    expect(rows).toHaveLength(1);
    expect(rows[0]?.lpId).toBe("XTX Markets");
    // An unknown lp id yields no rows (honest empty, never a fabricated row).
    expect(await t.listLpFlowMetrics(undefined, "Nonexistent LP")).toEqual([]);
  });
});

// --- capability model + tab gating ------------------------------------------

function authWith(caps: string[], isAdmin = false): NavAuth {
  const held = new Set(caps);
  return { isAdmin, can: (action, asset) => held.has(`${action}·${asset}`) };
}

describe("Street Liquidity tab gating", () => {
  it("streetliquidity is a member of ANALYTICS_WORKSPACES", () => {
    expect(ANALYTICS_WORKSPACES.has("streetliquidity")).toBe(true);
  });

  it("the tab + workspace are HIDDEN for a signed-in user without view_analytics", () => {
    const trader = authWith(["view·fx_options", "view·fixed_income", "execute·fixed_income"]);
    expect(domainAccessible("analytics", trader)).toBe(false);
    expect(workspaceAccessible("streetliquidity", trader)).toBe(false);
  });

  it("holding view_analytics on EITHER asset admits the workspace (cross-product OR)", () => {
    const fxOnly = authWith(["view·fx_options", "view_analytics·fx_options"]);
    expect(workspaceAccessible("streetliquidity", fxOnly)).toBe(true);

    const fiOnly = authWith(["view·fixed_income", "view_analytics·fixed_income"]);
    expect(workspaceAccessible("streetliquidity", fiOnly)).toBe(true);
  });

  it("an admin (grant-all) reaches it", () => {
    const admin: NavAuth = { isAdmin: true, can: () => true };
    expect(workspaceAccessible("streetliquidity", admin)).toBe(true);
  });
});
