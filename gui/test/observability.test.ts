/**
 * W12 GUI capstone — server observability + typed smile provenance.
 *
 * Exercises the REAL contract seam end-to-end with NO server and NO mocks of the
 * code under test:
 *  - `heartbeatFromWire` decodes the server's additive observability JSON
 *    (`crates/celnet-server/src/ws/codec.rs` `Message::Heartbeat`) — conflation
 *    drops, drain-side price p50/p99/p99.9 (ns), and the surface/correlation echo.
 *  - `arbReportFromWire` decodes the TYPED, authoritative `smile_model` provenance
 *    field — the migration target that retires the `model=<family>` note regex.
 *  - `distillObservability` folds the per-subscription beats into the one
 *    ServerObservability the StatusRibbon renders (sum drops, worst p99).
 *
 * Provenance tags are HAND-PINNED to the wire numbers (0..=4), independent of the
 * codec's own enum table, so a silent off-by-one in `enums.ts` is caught (Lesson c).
 */
import { describe, expect, it } from "vitest";

import {
  arbReportFromWire,
  heartbeatFromWire,
  type WireObject,
} from "../src/data/wsCodec";
import {
  distillObservability,
  type ServerObservability,
} from "../src/hooks/useStreamSession";
import type { Heartbeat, SmileModel } from "../src/data/contract";

// ---------------------------------------------------------------------------
// heartbeatFromWire — the server liveness beat with additive observability
// ---------------------------------------------------------------------------

describe("heartbeatFromWire — server observability decode", () => {
  // The exact JSON shape the server emits (codec.rs `Message::Heartbeat`).
  const wire: WireObject = {
    subscription: { value: 7 },
    sequence: 4242,
    epoch_nanos: 1_700_000_000_000_000_000,
    conflation_drops: 19,
    server_price_p50_nanos: 850,
    server_price_p99_nanos: 3100,
    server_price_p999_nanos: 9700,
    surface_version: 3,
    correlation_id: 88,
  };

  it("recovers every observability field as a bigint, exactly", () => {
    const h = heartbeatFromWire(wire);
    expect(h.subscriptionId).toBe(7n);
    expect(h.sequence).toBe(4242n);
    expect(h.conflationDrops).toBe(19n);
    expect(h.serverPriceP50Nanos).toBe(850n);
    expect(h.serverPriceP99Nanos).toBe(3100n);
    expect(h.serverPriceP999Nanos).toBe(9700n);
    expect(h.surfaceVersion).toBe(3n);
    expect(h.correlationId).toBe(88n);
    expect(h.epochNanos).toBe(1_700_000_000_000_000_000n);
  });

  it("treats absent provenance echo as honestly undefined (not 0)", () => {
    const h = heartbeatFromWire({
      subscription: { value: 1 },
      sequence: 1,
      epoch_nanos: 1,
      conflation_drops: 0,
      server_price_p50_nanos: 0,
      server_price_p99_nanos: 0,
      server_price_p999_nanos: 0,
    });
    expect(h.surfaceVersion).toBeUndefined();
    expect(h.correlationId).toBeUndefined();
    // A never-timed line reports a real zero latency / zero drops (not undefined).
    expect(h.conflationDrops).toBe(0n);
    expect(h.serverPriceP99Nanos).toBe(0n);
  });

  it("distinguishes an explicit 0 echo (live/unpinned) from absence", () => {
    const h = heartbeatFromWire({
      subscription: { value: 1 },
      sequence: 1,
      epoch_nanos: 1,
      conflation_drops: 0,
      server_price_p50_nanos: 0,
      server_price_p99_nanos: 0,
      server_price_p999_nanos: 0,
      surface_version: 0,
      correlation_id: 0,
    });
    expect(h.surfaceVersion).toBe(0n);
    expect(h.correlationId).toBe(0n);
  });
});

// ---------------------------------------------------------------------------
// arbReportFromWire — TYPED smile-model provenance (the regex→field migration)
// ---------------------------------------------------------------------------

describe("arbReportFromWire — typed smile-model provenance", () => {
  // HAND-PINNED to the proto wire numbers, independent of enums.ts (Lesson c).
  const PINNED: [number, SmileModel][] = [
    [0, "MARKET_HEDGE"],
    [1, "STOCHASTIC_VOL"],
    [2, "PARAMETRIC"],
    [3, "PARAMETRIC_SURFACE"],
    [4, "EXTENDED_SURFACE"],
  ];

  it("decodes the typed model field for every wire tag (0..=4)", () => {
    for (const [tag, model] of PINNED) {
      const a = arbReportFromWire({
        butterfly_arbitrage_free: true,
        calendar_arbitrage_free: true,
        worst_density: 0,
        note: "arb-free",
        smile_model: tag,
      });
      expect(a.model).toBe(model);
    }
  });

  it("reads the TYPED field, never the note's human model= token", () => {
    // The note lies (says market-hedge); the typed field says eSSVI. The typed
    // field wins — proving code reads `smile_model`, not a note regex.
    const a = arbReportFromWire({
      butterfly_arbitrage_free: true,
      calendar_arbitrage_free: true,
      worst_density: 0,
      note: "arb-free · model=market-hedge",
      smile_model: 4,
    });
    expect(a.model).toBe("EXTENDED_SURFACE");
  });

  it("defaults an absent smile_model to MARKET_HEDGE (proto3 enum-zero)", () => {
    const a = arbReportFromWire({
      butterfly_arbitrage_free: true,
      calendar_arbitrage_free: true,
      worst_density: 0,
      note: "arb-free",
    });
    expect(a.model).toBe("MARKET_HEDGE");
  });
});

// ---------------------------------------------------------------------------
// distillObservability — fold per-subscription beats into the ribbon's view
// ---------------------------------------------------------------------------

function beat(over: Partial<Heartbeat>): Heartbeat {
  return {
    subscriptionId: 1n,
    sequence: 1n,
    conflationDrops: 0n,
    serverPriceP50Nanos: 0n,
    serverPriceP99Nanos: 0n,
    serverPriceP999Nanos: 0n,
    epochNanos: 1n,
    ...over,
  };
}

describe("distillObservability — per-subscription beats → one ribbon view", () => {
  it("reports the honest empty-state before any beat", () => {
    const obs = distillObservability(new Map());
    expect(obs.received).toBe(false);
    expect(obs.conflationDrops).toBe(0n);
    expect(obs.serverPriceP99Nanos).toBe(0n);
    expect(obs.surfaceVersion).toBeUndefined();
  });

  it("SUMS conflation drops and takes the WORST (max) p99 across lines", () => {
    const beats = new Map<bigint, Heartbeat>([
      [1n, beat({ subscriptionId: 1n, conflationDrops: 5n, serverPriceP99Nanos: 1200n, epochNanos: 10n })],
      [2n, beat({ subscriptionId: 2n, conflationDrops: 11n, serverPriceP99Nanos: 4800n, epochNanos: 20n })],
      [3n, beat({ subscriptionId: 3n, conflationDrops: 0n, serverPriceP99Nanos: 900n, epochNanos: 15n })],
    ]);
    const obs = distillObservability(beats);
    expect(obs.received).toBe(true);
    // Hand-pinned: 5 + 11 + 0 = 16 total drops; worst p99 = 4800ns.
    expect(obs.conflationDrops).toBe(16n);
    expect(obs.serverPriceP99Nanos).toBe(4800n);
  });

  it("takes the provenance echo from the most-recently-beating line", () => {
    const beats = new Map<bigint, Heartbeat>([
      [1n, beat({ subscriptionId: 1n, epochNanos: 10n, surfaceVersion: 2n, correlationId: 7n })],
      // The latest beat (epoch 30) carries the authoritative echo.
      [2n, beat({ subscriptionId: 2n, epochNanos: 30n, surfaceVersion: 9n, correlationId: 0n })],
      [3n, beat({ subscriptionId: 3n, epochNanos: 20n, surfaceVersion: 5n, correlationId: 3n })],
    ]);
    const obs = distillObservability(beats);
    expect(obs.surfaceVersion).toBe(9n);
    expect(obs.correlationId).toBe(0n);
  });

  it("omits the provenance echo when no line carries one (honest undefined)", () => {
    const beats = new Map<bigint, Heartbeat>([[1n, beat({ subscriptionId: 1n })]]);
    const obs: ServerObservability = distillObservability(beats);
    expect(obs.received).toBe(true);
    expect(obs.surfaceVersion).toBeUndefined();
    expect(obs.correlationId).toBeUndefined();
  });
});
