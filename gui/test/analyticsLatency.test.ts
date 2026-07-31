/**
 * Latency / Ops analytics — the codec round-trip, the offline mock data shape, and
 * the capability/domain gating of the Latency workspace on the Analytics tab. Pure
 * (no React); the workspace render/format lives in analyticsLatencyWorkspace.test.tsx.
 */
import { describe, expect, it } from "vitest";

import {
  latencyStageFromWire,
  latencyHealthFromWire,
  listLatencyMetricsRequestToWire,
  listLatencyMetricsResponseFromWire,
} from "../src/data/wsCodec";
import { MockTransport } from "../src/data/mockSource";
import {
  ANALYTICS_WORKSPACES,
  DOMAINS,
  domainAccessible,
  workspaceAccessible,
  type NavAuth,
} from "../src/lib/commands";
import type { WireObject } from "../src/data/wsCodec";

// The stages the server emits, in order (contract-locked).
const EXPECTED_OPS = [
  "vanilla_price",
  "surface_vol",
  "tiering_run",
  "consolidate",
  "stream_publish",
  "rfq_respond",
  "quote_accept",
  "book",
] as const;

// --- request framing --------------------------------------------------------

describe("listLatencyMetricsRequestToWire", () => {
  it("frames an empty body (the session token is auto-injected by the WS conn)", () => {
    expect(listLatencyMetricsRequestToWire()).toEqual({});
  });
});

// --- response decode (snake_case → camelCase) -------------------------------

/** A fully-populated wire stage row. */
function fullWireStage(): WireObject {
  return {
    op: "vanilla_price",
    stage_label: "Price (pinned core)",
    count: 12_345,
    p50_ns: 820,
    p99_ns: 2_400,
    p999_ns: 5_200,
    p9999_ns: 9_100,
    min_ns: 240,
    max_ns: 14_000,
    mean_ns: 910.4,
  };
}

describe("latencyStageFromWire", () => {
  it("maps every snake_case wire key to its camelCase field", () => {
    const s = latencyStageFromWire(fullWireStage());
    expect(s.op).toBe("vanilla_price");
    expect(s.stageLabel).toBe("Price (pinned core)");
    expect(s.count).toBe(12_345);
    expect(s.p50Ns).toBe(820);
    expect(s.p99Ns).toBe(2_400);
    expect(s.p999Ns).toBe(5_200);
    expect(s.p9999Ns).toBe(9_100);
    expect(s.minNs).toBe(240);
    expect(s.maxNs).toBe(14_000);
    expect(s.meanNs).toBeCloseTo(910.4);
  });

  it("decodes an unsampled stage (missing numeric keys) to zeros, never NaN", () => {
    const s = latencyStageFromWire({ op: "book", stage_label: "Ack→fill→book" });
    expect(s.op).toBe("book");
    expect(s.count).toBe(0);
    expect(s.p50Ns).toBe(0);
    expect(s.maxNs).toBe(0);
    expect(s.meanNs).toBe(0);
    for (const v of [s.count, s.p50Ns, s.p99Ns, s.p999Ns, s.p9999Ns, s.minNs, s.maxNs, s.meanNs]) {
      expect(Number.isNaN(v)).toBe(false);
    }
  });
});

describe("latencyHealthFromWire", () => {
  it("decodes the telemetry offload-queue digest", () => {
    const h = latencyHealthFromWire({
      drained_total: 100_000,
      dropped_total: 12,
      observed_gaps: 0,
      tick_hz: 24_000_000,
    });
    expect(h).toEqual({ drainedTotal: 100_000, droppedTotal: 12, observedGaps: 0, tickHz: 24_000_000 });
  });

  it("defaults a missing health envelope to zeros", () => {
    expect(latencyHealthFromWire({})).toEqual({
      drainedTotal: 0,
      droppedTotal: 0,
      observedGaps: 0,
      tickHz: 0,
    });
  });
});

describe("listLatencyMetricsResponseFromWire", () => {
  it("decodes the { stages, health } envelope preserving stage order", () => {
    const wire: WireObject = {
      stages: [
        fullWireStage(),
        { op: "book", stage_label: "Ack→fill→book", count: 41_000, p50_ns: 2_600_000, p99_ns: 6_100_000, p999_ns: 9_400_000, p9999_ns: 13_000_000, min_ns: 1_100_000, max_ns: 17_000_000, mean_ns: 2_980_000 },
      ],
      health: { drained_total: 100_000, dropped_total: 12, observed_gaps: 0, tick_hz: 24_000_000 },
    };
    const m = listLatencyMetricsResponseFromWire(wire);
    expect(m.stages.map((s) => s.op)).toEqual(["vanilla_price", "book"]);
    expect(m.stages[0]?.p99Ns).toBe(2_400);
    expect(m.health.tickHz).toBe(24_000_000);
  });

  it("decodes an empty/absent envelope to no stages + zero health", () => {
    const m = listLatencyMetricsResponseFromWire({});
    expect(m.stages).toEqual([]);
    expect(m.health).toEqual({ drainedTotal: 0, droppedTotal: 0, observedGaps: 0, tickHz: 0 });
  });
});

// --- offline mock data shape ------------------------------------------------

describe("MockTransport.listLatencyMetrics", () => {
  const t = new MockTransport();

  it("emits all 8 instrumented stages in the server's pipeline order", async () => {
    const m = await t.listLatencyMetrics();
    expect(m.stages.map((s) => s.op)).toEqual([...EXPECTED_OPS]);
    // Each stage carries its human title and a plausible sample count.
    for (const s of m.stages) {
      expect(s.stageLabel.length).toBeGreaterThan(0);
      expect(s.count).toBeGreaterThan(0);
    }
  });

  it("has monotone percentiles (min ≤ p50 ≤ p99 ≤ p99.9 ≤ p99.99 ≤ max) per stage", async () => {
    const m = await t.listLatencyMetrics();
    for (const s of m.stages) {
      expect(s.minNs).toBeLessThanOrEqual(s.p50Ns);
      expect(s.p50Ns).toBeLessThanOrEqual(s.p99Ns);
      expect(s.p99Ns).toBeLessThanOrEqual(s.p999Ns);
      expect(s.p999Ns).toBeLessThanOrEqual(s.p9999Ns);
      expect(s.p9999Ns).toBeLessThanOrEqual(s.maxNs);
      expect(Number.isNaN(s.meanNs)).toBe(false);
    }
  });

  it("places the pinned core sub-µs and the book round-trip in the ms", async () => {
    const m = await t.listLatencyMetrics();
    const price = m.stages.find((s) => s.op === "vanilla_price");
    const book = m.stages.find((s) => s.op === "book");
    expect(price?.p50Ns).toBeLessThan(1_000); // sub-microsecond median
    expect(book?.p50Ns).toBeGreaterThan(1_000_000); // milliseconds
  });

  it("reports a healthy offload queue: a few drops, no gaps, 24 MHz tick", async () => {
    const m = await t.listLatencyMetrics();
    expect(m.health.tickHz).toBe(24_000_000);
    expect(m.health.droppedTotal).toBeGreaterThan(0);
    expect(m.health.observedGaps).toBe(0);
    expect(m.health.drainedTotal).toBeGreaterThan(0);
  });

  it("returns a fresh copy each call (the caller cannot mutate the fixture)", async () => {
    const a = await t.listLatencyMetrics();
    a.stages[0]!.p50Ns = -1;
    const b = await t.listLatencyMetrics();
    expect(b.stages[0]?.p50Ns).toBeGreaterThan(0);
  });
});

// --- capability model + tab gating (mirrors analyticsClientFlow) -------------

function authWith(caps: string[], isAdmin = false): NavAuth {
  const held = new Set(caps);
  return { isAdmin, can: (action, asset) => held.has(`${action}·${asset}`) };
}

describe("Latency / Ops tab gating", () => {
  it("DOMAINS carries an analytics tab between the trading tabs and Administration", () => {
    const ids = DOMAINS.map((d) => d.id);
    expect(ids).toContain("analytics");
    expect(ids.indexOf("analytics")).toBe(ids.indexOf("admin") - 1);
  });

  it("latencyops is a member of ANALYTICS_WORKSPACES", () => {
    expect(ANALYTICS_WORKSPACES.has("latencyops")).toBe(true);
  });

  it("the tab + workspace are HIDDEN for a signed-in user without view_analytics", () => {
    const trader = authWith(["view·fx_options", "view·fixed_income", "execute·fixed_income"]);
    expect(domainAccessible("analytics", trader)).toBe(false);
    expect(workspaceAccessible("latencyops", trader)).toBe(false);
  });

  it("holding view_analytics on EITHER asset admits the workspace (cross-product OR)", () => {
    const fxOnly = authWith(["view·fx_options", "view_analytics·fx_options"]);
    expect(domainAccessible("analytics", fxOnly)).toBe(true);
    expect(workspaceAccessible("latencyops", fxOnly)).toBe(true);

    const fiOnly = authWith(["view·fixed_income", "view_analytics·fixed_income"]);
    expect(domainAccessible("analytics", fiOnly)).toBe(true);
    expect(workspaceAccessible("latencyops", fiOnly)).toBe(true);
  });

  it("an admin (grant-all in practice) reaches it", () => {
    const admin: NavAuth = { isAdmin: true, can: () => true };
    expect(domainAccessible("analytics", admin)).toBe(true);
    expect(workspaceAccessible("latencyops", admin)).toBe(true);
  });
});
