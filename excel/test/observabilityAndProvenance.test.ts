import { describe, expect, it } from "vitest";
import {
  arbReportFromWire,
  heartbeatFromWire,
  smileFromWire,
} from "../src/contract/wsCodec";
import { Connection, type StreamEvent } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import { formatServerStatusSpill, type SpillMatrix } from "../src/functions/shaping";
import type { Heartbeat, SmileModel } from "../src/contract/contract";

// ---------------------------------------------------------------------------
// Independent oracle (Lesson c): pin the wire SmileModel tags to HAND values,
// not derived from the codec under test. This is the proto3 numbering of
// `celnet_proto::SmileModel` (SMILE_MODEL_MARKET_HEDGE=0, … EXTENDED_SURFACE=4),
// transcribed by hand from the server's arb_report_to_json / smile_model_label.
// The decoder is correct iff it maps each of these tags to the matching family.
// ---------------------------------------------------------------------------
const SMILE_MODEL_ORACLE: ReadonlyArray<readonly [number, SmileModel, string]> = [
  [0, "MARKET_HEDGE", "market-hedge"],
  [1, "STOCHASTIC_VOL", "stochastic-vol"],
  [2, "PARAMETRIC", "parametric"],
  [3, "PARAMETRIC_SURFACE", "parametric-surface"],
  [4, "EXTENDED_SURFACE", "extended-surface"],
];

/** Build the EXACT JSON the server's `arb_report_to_json` emits for a given tag. */
function serverArbReportJson(tag: number, label: string): Record<string, unknown> {
  return {
    butterfly_arbitrage_free: true,
    calendar_arbitrage_free: true,
    worst_density: 0,
    // The note still embeds the legacy human-only token, which must be IGNORED.
    note: "calibrated; model=market_hedge",
    smile_model: tag,
    smile_model_label: label,
  };
}

describe("typed smile-model provenance (arbReportFromWire)", () => {
  it("decodes every wire tag to the family pinned by the hand oracle", () => {
    for (const [tag, family] of SMILE_MODEL_ORACLE) {
      const arb = arbReportFromWire(serverArbReportJson(tag, "ignored"));
      expect(arb.smileModel).toBe(family);
    }
  });

  it("reads the TYPED field, never the legacy `model=` note token", () => {
    // The note says market_hedge; the typed tag says EXTENDED_SURFACE(4). The
    // decoder must trust the typed field.
    const arb = arbReportFromWire({
      butterfly_arbitrage_free: true,
      calendar_arbitrage_free: true,
      worst_density: 0,
      note: "calibrated; model=market_hedge",
      smile_model: 4,
      smile_model_label: "extended-surface",
    });
    expect(arb.smileModel).toBe("EXTENDED_SURFACE");
    // The note is preserved verbatim for human eyes but is NOT authoritative.
    expect(arb.note).toBe("calibrated; model=market_hedge");
  });

  it("treats an absent smile_model as the proto3 zero value (MARKET_HEDGE)", () => {
    const arb = arbReportFromWire({
      butterfly_arbitrage_free: true,
      calendar_arbitrage_free: true,
      worst_density: 0,
      note: "",
    });
    expect(arb.smileModel).toBe("MARKET_HEDGE");
  });

  it("surfaces the typed provenance through a full server-shaped smile frame", () => {
    // The exact `smile_to_json` shape (eSSVI-marked tenor).
    const smile = smileFromWire({
      pair: { base: "EUR", quote: "USD" },
      tenor_years: 1.0,
      broker_quotes: null,
      points: [
        { delta: -0.25, tenor_years: 1.0, vol: 0.108 },
        { delta: 0.25, tenor_years: 1.0, vol: 0.112 },
      ],
      conventions: {},
      arbitrage: serverArbReportJson(4, "extended-surface"),
      epoch_nanos: 0,
    });
    expect(smile.arbitrage.smileModel).toBe("EXTENDED_SURFACE");
    expect(smile.points).toHaveLength(2);
  });
});

// ---------------------------------------------------------------------------
// Heartbeat observability decode — against the EXACT server `heartbeat` frame
// (crates/celnet-server/src/ws/codec.rs server_stream_message_to_json).
// ---------------------------------------------------------------------------
describe("heartbeat observability (heartbeatFromWire)", () => {
  function serverHeartbeat(extra: Record<string, unknown>): Record<string, unknown> {
    return {
      type: "heartbeat",
      subscription: null,
      sequence: 12,
      epoch_nanos: 1700,
      conflation_drops: 0,
      server_price_p50_nanos: 0,
      server_price_p99_nanos: 0,
      server_price_p999_nanos: 0,
      surface_version: 0,
      correlation_id: 0,
      ...extra,
    };
  }

  it("decodes a connection-level beat (subscription null ⇒ subscriptionId undefined)", () => {
    const hb = heartbeatFromWire(serverHeartbeat({}));
    expect(hb.subscriptionId).toBeUndefined();
    expect(hb.sequence).toBe(12n);
    expect(hb.conflationDrops).toBe(0n);
    expect(hb.serverPriceP50Nanos).toBe(0n);
  });

  it("decodes a per-subscription beat with real observability values", () => {
    const hb = heartbeatFromWire(
      serverHeartbeat({
        subscription: { value: 7 },
        conflation_drops: 42,
        server_price_p50_nanos: 1234,
        server_price_p99_nanos: 9876,
        server_price_p999_nanos: 54321,
        surface_version: 9,
        correlation_id: 3,
      }),
    );
    expect(hb.subscriptionId).toBe(7n);
    expect(hb.conflationDrops).toBe(42n);
    expect(hb.serverPriceP50Nanos).toBe(1234n);
    expect(hb.serverPriceP99Nanos).toBe(9876n);
    expect(hb.serverPriceP999Nanos).toBe(54321n);
    expect(hb.surfaceVersion).toBe(9n);
    expect(hb.correlationId).toBe(3n);
  });
});

// ---------------------------------------------------------------------------
// formatServerStatusSpill — the CELNET.STATUS layout, with an independent
// hand-computed expectation for the ns→µs rendering (Lesson c).
// ---------------------------------------------------------------------------
function row(spill: SpillMatrix, r: number): ReadonlyArray<string | number> {
  const out = spill[r];
  if (!out) throw new Error(`no row ${r}`);
  return out;
}

describe("formatServerStatusSpill", () => {
  it("renders an awaiting state with no beat seen yet", () => {
    const spill = formatServerStatusSpill(true, undefined);
    expect(spill).toHaveLength(2);
    expect(row(spill, 0)).toEqual([
      "connection",
      "price_p50",
      "price_p99",
      "price_p99.9",
      "conflation_drops",
      "surface",
      "correlation",
    ]);
    expect(row(spill, 1)[0]).toBe("LIVE");
    expect(row(spill, 1)[4]).toBe("(awaiting beat)");
  });

  it("renders DOWN when the socket is not open", () => {
    const spill = formatServerStatusSpill(false, undefined);
    expect(row(spill, 1)[0]).toBe("DOWN");
  });

  it("renders the server's percentiles as µs and the exact drop count", () => {
    // Hand oracle: 1500ns = 1.50µs; 9999ns → (9999*100)/1000 = 999 hundredths
    // → 9.99µs; 54321ns → 5432199/... → 5432 hundredths → 54.32µs; 0ns → "—".
    const hb: Heartbeat = {
      sequence: 5n,
      conflationDrops: 7n,
      serverPriceP50Nanos: 1500n,
      serverPriceP99Nanos: 9999n,
      serverPriceP999Nanos: 54321n,
      surfaceVersion: 0n,
      correlationId: 0n,
      epochNanos: 0n,
    };
    const spill = formatServerStatusSpill(true, hb);
    const values = row(spill, 1);
    expect(values[0]).toBe("LIVE");
    expect(values[1]).toBe("1.50µs");
    expect(values[2]).toBe("9.99µs");
    expect(values[3]).toBe("54.32µs");
    expect(values[4]).toBe("7"); // exact ring skip count
    expect(values[5]).toBe("live"); // surface_version 0 ⇒ live/unpinned
    expect(values[6]).toBe("—"); // correlation 0 ⇒ none
  });

  it("renders a pinned surface version and correlation id when present", () => {
    const hb: Heartbeat = {
      sequence: 5n,
      conflationDrops: 0n,
      serverPriceP50Nanos: 0n,
      serverPriceP99Nanos: 0n,
      serverPriceP999Nanos: 0n,
      surfaceVersion: 42n,
      correlationId: 99n,
      epochNanos: 0n,
    };
    const values = row(formatServerStatusSpill(true, hb), 1);
    expect(values[1]).toBe("—"); // zero latency ⇒ honest waiting marker, not 0.00µs
    expect(values[5]).toBe("v42");
    expect(values[6]).toBe("99");
  });
});

// ---------------------------------------------------------------------------
// End-to-end: the live Connection surfaces the beat as an event AND caches the
// latest for CELNET.STATUS to read — against a real WS-shaped frame.
// ---------------------------------------------------------------------------
class FakeSocket implements WebSocketLike {
  readyState = 0;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((data: string) => void) | null = null;
  send(): void {}
  close(): void {
    this.readyState = 3;
    this.onclose?.();
  }
  open(): void {
    this.readyState = 1;
    this.onopen?.();
  }
  deliver(frame: Record<string, unknown>): void {
    this.onmessage?.(JSON.stringify(frame));
  }
}

describe("Connection surfaces heartbeat observability", () => {
  it("emits a heartbeat event and caches the latest beat for CELNET.STATUS", () => {
    const sock = new FakeSocket();
    const conn = new Connection({
      url: "ws://test",
      factory: () => sock,
      stalenessWindowMs: 0, // disable the monitor; this test only drives frames
      requestTimeoutMs: 5000,
    });
    const events: StreamEvent[] = [];
    conn.onEvent((e) => events.push(e));
    sock.open();
    expect(conn.latestHeartbeat()).toBeUndefined();

    sock.deliver({
      type: "heartbeat",
      subscription: null,
      sequence: 3,
      epoch_nanos: 0,
      conflation_drops: 11,
      server_price_p50_nanos: 2000,
      server_price_p99_nanos: 8000,
      server_price_p999_nanos: 40000,
      surface_version: 0,
      correlation_id: 0,
    });

    const beats = events.filter((e) => e.kind === "heartbeat");
    expect(beats).toHaveLength(1);
    const cached = conn.latestHeartbeat();
    expect(cached?.conflationDrops).toBe(11n);
    expect(cached?.serverPriceP50Nanos).toBe(2000n);

    // A later beat replaces the cached one (the desk always reads the newest).
    sock.deliver({
      type: "heartbeat",
      subscription: null,
      sequence: 4,
      epoch_nanos: 0,
      conflation_drops: 11,
      server_price_p50_nanos: 2100,
      server_price_p99_nanos: 8100,
      server_price_p999_nanos: 41000,
      surface_version: 0,
      correlation_id: 0,
    });
    expect(conn.latestHeartbeat()?.serverPriceP50Nanos).toBe(2100n);
  });
});
