import { describe, expect, it } from "vitest";
import { Connection, type StreamEvent } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import {
  RatesStreamRegistry,
  ratesKey,
  type RatesLiveTick,
  type RatesStreamConnection,
} from "../src/functions/ratesStreamRegistry";
import {
  formatRatesSeriesCell,
  parseRatesObservable,
  shapeOisInstrument,
  shapeRatesCurve,
  ShapingError,
} from "../src/functions/shaping";
import {
  oisRatesInstrument,
  type RatesCurveSet,
  type RatesInstrument,
  type RatesPricingResult,
  type RatesStreamSnapshot,
  type RatesStreamUpdate,
} from "../src/contract/contract";
import {
  ratesCurveSetToWire,
  ratesInstrumentUnionToWire,
  ratesStreamSnapshotFromWire,
  ratesStreamUpdateFromWire,
} from "../src/contract/wsCodec";

// ---------------------------------------------------------------------------
// Shared fixtures: an OIS priced against a self-discounting curve — the exact
// instrument CELNET.RATES / CELNET.RATESSERIES shape.
// ---------------------------------------------------------------------------

const CURVE: RatesCurveSet = shapeRatesCurve({
  curve: [
    [1, 0.038],
    [2, 0.039],
    [5, 0.041],
    [10, 0.043],
  ],
  referenceDate: "2026-07-03",
  currency: "USD",
});

const OIS = shapeOisInstrument({ tenor: "5Y", fixedRate: 0.041, direction: "PAY_FIXED", notional: 1_000_000 });
const INSTR: RatesInstrument = oisRatesInstrument(OIS);

function result(pv: number): RatesPricingResult {
  return { pv, parRate: 0.0412, pv01: 480.5, dv01: 479.9, keyRateLadder: [90.1, 120.2, 150.3, 119.3] };
}

// ---------------------------------------------------------------------------
// codec byte-shape: encode the union arm the server `rates_instrument_from_json`
// reads, and decode the server-shaped `rates_stream_snapshot/update` frames. The
// frames below are constructed EXACTLY as the server encodes them
// (crates/celnet-server/src/ws/codec.rs: rates_stream_snapshot_to_json /
// rates_stream_update_to_json + rates_pricing_result_to_json), so a passing decode
// is a byte-shape conformance against the server WS mirror.
// ---------------------------------------------------------------------------

describe("rates streaming wire codec (byte-shape vs the server WS mirror)", () => {
  it("encodes the OIS union arm exactly as rates_instrument_from_json reads it", () => {
    // `ratesInstrumentUnionToWire` must produce the `{ ois: {...} }` oneof the server
    // decodes — flat scalars + numeric `Side` (PAY_FIXED → SIDE_BUY = 0).
    expect(ratesInstrumentUnionToWire(INSTR)).toEqual({
      ois: { tenor_years: 5, fixed_rate: 0.041, notional: 1_000_000, side: 0 },
    });
    // The RECEIVE_FIXED arm flips the numeric side to SIDE_SELL = 1.
    const recv = oisRatesInstrument(
      shapeOisInstrument({ tenor: 5, fixedRate: 0.041, direction: "RECEIVE_FIXED", notional: 1_000_000 }),
    );
    expect(ratesInstrumentUnionToWire(recv)).toEqual({
      ois: { tenor_years: 5, fixed_rate: 0.041, notional: 1_000_000, side: 1 },
    });
  });

  it("the curve_set the rates_subscribe frame carries is the nested-tenor shape the server reads", () => {
    expect(ratesCurveSetToWire(CURVE)).toEqual({
      currency: "USD",
      reference_date: { year: 2026, month: 7, day: 3 },
      ois_pillars: [
        { tenor: { years: 1 }, par_rate: 0.038 },
        { tenor: { years: 2 }, par_rate: 0.039 },
        { tenor: { years: 5 }, par_rate: 0.041 },
        { tenor: { years: 10 }, par_rate: 0.043 },
      ],
    });
  });

  it("decodes a server-shaped rates_stream_snapshot (result == price_rates shape) with the correlation echo", () => {
    const frame = {
      type: "rates_stream_snapshot",
      subscription: { value: 7 },
      sequence: 1,
      result: { pv: 1234.5, par_rate: 0.0412, pv01: 480.5, dv01: 479.9, key_rate_ladder: [90.1, 120.2, 150.3, 119.3] },
      curve_shift: 0,
      correlation_id: 99,
      epoch_nanos: 1_700_000_000_000_000_000,
    };
    const snap: RatesStreamSnapshot = ratesStreamSnapshotFromWire(frame);
    expect(snap.subscriptionId).toBe(7n);
    expect(snap.sequence).toBe(1n);
    expect(snap.result.pv).toBeCloseTo(1234.5, 6);
    expect(snap.result.parRate).toBeCloseTo(0.0412, 8);
    expect(snap.result.pv01).toBeCloseTo(480.5, 6);
    expect(snap.result.dv01).toBeCloseTo(479.9, 6);
    expect(snap.result.keyRateLadder).toEqual([90.1, 120.2, 150.3, 119.3]);
    expect(snap.curveShift).toBe(0);
    expect(snap.correlationId).toBe(99n);
    expect(snap.epochNanos).toBe(1_700_000_000_000_000_000n);
  });

  it("treats an absent/null snapshot correlation_id as none (presence-tracked)", () => {
    const frame = {
      type: "rates_stream_snapshot",
      subscription: { value: 3 },
      sequence: 1,
      result: { pv: 0, par_rate: 0.04, pv01: 0, dv01: 0, key_rate_ladder: [] },
      curve_shift: 0,
      correlation_id: null,
      epoch_nanos: 1,
    };
    expect(ratesStreamSnapshotFromWire(frame).correlationId).toBeUndefined();
  });

  it("decodes a server-shaped rates_stream_update carrying the shifted-curve reprice + curve_shift", () => {
    const frame = {
      type: "rates_stream_update",
      subscription: { value: 7 },
      sequence: 2,
      result: { pv: 1300.25, par_rate: 0.0413, pv01: 481.0, dv01: 480.4, key_rate_ladder: [91, 121, 151, 118] },
      curve_shift: 0.0001,
      // A 64-bit nanos value beyond JS safe-integer range, carried as an exact
      // `bigint` (the lossless `parseFrame` path yields these for oversized ints).
      epoch_nanos: 1_700_000_000_000_000_500n,
    };
    const upd: RatesStreamUpdate = ratesStreamUpdateFromWire(frame);
    expect(upd.subscriptionId).toBe(7n);
    expect(upd.sequence).toBe(2n);
    expect(upd.result.pv).toBeCloseTo(1300.25, 6);
    expect(upd.curveShift).toBeCloseTo(0.0001, 8);
    expect(upd.epochNanos).toBe(1_700_000_000_000_000_500n);
  });
});

// ---------------------------------------------------------------------------
// registry: coalescing / refcount / latest-tick / observable-independence.
// A scriptable in-memory connection: records subscribe/unsubscribe and lets a test
// push server events. No server, no mock of OUR functionality — just the transport
// seam the registry depends on.
// ---------------------------------------------------------------------------

class FakeRatesConnection implements RatesStreamConnection {
  private nextId = 1n;
  readonly subscribed: { id: bigint; instrument: RatesInstrument; curve: RatesCurveSet }[] = [];
  readonly unsubscribed: bigint[] = [];
  private listener: ((e: StreamEvent) => void) | null = null;

  subscribeRates(instrument: RatesInstrument, curve: RatesCurveSet): bigint {
    const id = this.nextId++;
    this.subscribed.push({ id, instrument, curve });
    return id;
  }
  unsubscribeRates(id: bigint): void {
    this.unsubscribed.push(id);
  }
  onEvent(listener: (e: StreamEvent) => void): () => void {
    this.listener = listener;
    return () => {
      this.listener = null;
    };
  }
  push(e: StreamEvent): void {
    this.listener?.(e);
  }
}

function snapshotEvent(subId: bigint, seq: bigint, pv: number): StreamEvent {
  return {
    kind: "rates_snapshot",
    snapshot: { subscriptionId: subId, sequence: seq, result: result(pv), curveShift: 0, epochNanos: 1n },
  };
}
function updateEvent(subId: bigint, seq: bigint, pv: number, shift: number): StreamEvent {
  return {
    kind: "rates_update",
    update: { subscriptionId: subId, sequence: seq, result: result(pv), curveShift: shift, epochNanos: 2n },
  };
}

describe("RatesStreamRegistry coalescing + refcount", () => {
  it("shares one server line across identical-argument cells and tears it down at zero", () => {
    const conn = new FakeRatesConnection();
    const reg = new RatesStreamRegistry(conn);
    const a = reg.acquire(INSTR, CURVE, "a", () => {});
    const b = reg.acquire(INSTR, CURVE, "b", () => {});
    expect(conn.subscribed.length).toBe(1); // ONE server line for two cells
    expect(reg.liveSubscriptionCount()).toBe(1);
    expect(reg.totalRefcount()).toBe(2);

    a.release();
    expect(conn.unsubscribed.length).toBe(0);
    expect(reg.totalRefcount()).toBe(1);
    b.release();
    expect(conn.unsubscribed).toEqual([conn.subscribed[0]!.id]);
    expect(reg.liveSubscriptionCount()).toBe(0);
  });

  it("a PV cell and a DV01 cell on the SAME curve+swap share ONE server line (observable is client-side)", () => {
    // The KEY design point: the streamed result carries every measure at once, so
    // the displayed observable is NOT part of the coalescing key.
    const conn = new FakeRatesConnection();
    const reg = new RatesStreamRegistry(conn);
    reg.acquire(INSTR, CURVE, "pv-cell", () => {});
    reg.acquire(INSTR, CURVE, "dv01-cell", () => {});
    expect(conn.subscribed.length).toBe(1);
    expect(reg.liveSubscriptionCount()).toBe(1);
  });

  it("opens distinct lines for distinct instruments and distinct curves", () => {
    const conn = new FakeRatesConnection();
    const reg = new RatesStreamRegistry(conn);
    const otherSwap = oisRatesInstrument(
      shapeOisInstrument({ tenor: "10Y", fixedRate: 0.043, direction: "PAY_FIXED", notional: 1_000_000 }),
    );
    const otherCurve = shapeRatesCurve({ curve: [[1, 0.03]], referenceDate: "2026-07-03", currency: "EUR" });
    reg.acquire(INSTR, CURVE, "a", () => {});
    reg.acquire(otherSwap, CURVE, "b", () => {});
    reg.acquire(INSTR, otherCurve, "c", () => {});
    expect(reg.liveSubscriptionCount()).toBe(3);
  });

  it("delivers snapshot then latest-tick updates (freshest reprice) to every shared cell", () => {
    const conn = new FakeRatesConnection();
    const reg = new RatesStreamRegistry(conn);
    const ticksA: RatesLiveTick[] = [];
    const ticksB: RatesLiveTick[] = [];
    reg.acquire(INSTR, CURVE, "a", (t) => ticksA.push(t));
    reg.acquire(INSTR, CURVE, "b", (t) => ticksB.push(t));
    const subId = conn.subscribed[0]!.id;

    conn.push(snapshotEvent(subId, 1n, 1234.5));
    conn.push(updateEvent(subId, 2n, 1300.25, 0.0001));

    expect(ticksA.at(-1)?.result?.pv).toBeCloseTo(1300.25, 6);
    expect(ticksA.at(-1)?.curveShift).toBeCloseTo(0.0001, 8);
    expect(ticksA.at(-1)?.health).toBe("HEALTHY");
    expect(ticksA.at(-1)?.baselined).toBe(true);
    expect(ticksB.at(-1)?.result?.pv).toBeCloseTo(1300.25, 6);
  });

  it("re-emits the last-good value under a STALE health (never frozen-as-live)", () => {
    const conn = new FakeRatesConnection();
    const reg = new RatesStreamRegistry(conn);
    const ticks: RatesLiveTick[] = [];
    reg.acquire(INSTR, CURVE, "a", (t) => ticks.push(t));
    const subId = conn.subscribed[0]!.id;
    conn.push(snapshotEvent(subId, 1n, 1234.5));
    conn.push({ kind: "health", subscriptionId: subId, health: "STALE" });
    expect(ticks.at(-1)?.result?.pv).toBeCloseTo(1234.5, 6); // last-good retained
    expect(ticks.at(-1)?.health).toBe("STALE");
  });

  it("ratesKey is observable-independent and separates instruments/curves", () => {
    expect(ratesKey(INSTR, CURVE)).toBe(ratesKey(INSTR, CURVE));
    const recv = oisRatesInstrument(
      shapeOisInstrument({ tenor: "5Y", fixedRate: 0.041, direction: "RECEIVE_FIXED", notional: 1_000_000 }),
    );
    expect(ratesKey(recv, CURVE)).not.toBe(ratesKey(INSTR, CURVE));
  });
});

// ---------------------------------------------------------------------------
// Connection: the real transport driving a rates line over a fake socket, the exact
// text JSON the server WS mirror speaks (mirrors connection.test.ts).
// ---------------------------------------------------------------------------

class FakeSocket implements WebSocketLike {
  readyState = 0;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((data: string) => void) | null = null;
  readonly sent: Record<string, unknown>[] = [];
  send(data: string): void {
    this.sent.push(JSON.parse(data) as Record<string, unknown>);
  }
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
  sentOfType(type: string): Record<string, unknown>[] {
    return this.sent.filter((f) => f["type"] === type);
  }
}

class ManualTime {
  now = 0;
  private seq = 1;
  private readonly timers = new Map<number, { fire: number; fn: () => void }>();
  clock = (): number => this.now;
  set = (fn: () => void, ms: number): unknown => {
    const id = this.seq++;
    this.timers.set(id, { fire: this.now + ms, fn });
    return id;
  };
  clear = (h: unknown): void => {
    this.timers.delete(h as number);
  };
  advance(ms: number): void {
    this.now += ms;
    for (const [id, t] of [...this.timers]) {
      if (t.fire <= this.now) {
        this.timers.delete(id);
        t.fn();
      }
    }
  }
}

function makeConn(): { conn: Connection; sock: FakeSocket; time: ManualTime; events: StreamEvent[] } {
  const sock = new FakeSocket();
  const time = new ManualTime();
  const conn = new Connection({
    url: "ws://test",
    factory: () => sock,
    stalenessWindowMs: 1000,
    clock: time.clock,
    setTimer: time.set,
    clearTimer: time.clear,
    requestTimeoutMs: 5000,
  });
  const events: StreamEvent[] = [];
  conn.onEvent((e) => events.push(e));
  return { conn, sock, time, events };
}

function serverSnapshotFrame(subId: number, seq: number, pv: number): Record<string, unknown> {
  return {
    type: "rates_stream_snapshot",
    subscription: { value: subId },
    sequence: seq,
    result: { pv, par_rate: 0.0412, pv01: 480.5, dv01: 479.9, key_rate_ladder: [90.1, 120.2, 150.3, 119.3] },
    curve_shift: 0,
    correlation_id: null,
    epoch_nanos: 1,
  };
}
function serverUpdateFrame(subId: number, seq: number, pv: number, shift: number): Record<string, unknown> {
  return {
    type: "rates_stream_update",
    subscription: { value: subId },
    sequence: seq,
    result: { pv, par_rate: 0.0413, pv01: 481, dv01: 480.4, key_rate_ladder: [91, 121, 151, 118] },
    curve_shift: shift,
    epoch_nanos: 2,
  };
}

describe("Connection fixed-income streaming line", () => {
  it("sends a rates_subscribe frame carrying the exact instrument-oneof + curve_set the server decodes", () => {
    const { conn, sock } = makeConn();
    sock.open();
    const id = conn.subscribeRates(INSTR, CURVE, "5Y OIS");
    const frames = sock.sentOfType("rates_subscribe");
    expect(frames.length).toBe(1);
    const f = frames[0]!;
    expect(f["subscription"]).toEqual({ value: Number(id) });
    expect(f["instrument"]).toEqual({ ois: { tenor_years: 5, fixed_rate: 0.041, notional: 1_000_000, side: 0 } });
    expect(f["curve_set"]).toEqual({
      currency: "USD",
      reference_date: { year: 2026, month: 7, day: 3 },
      ois_pillars: [
        { tenor: { years: 1 }, par_rate: 0.038 },
        { tenor: { years: 2 }, par_rate: 0.039 },
        { tenor: { years: 5 }, par_rate: 0.041 },
        { tenor: { years: 10 }, par_rate: 0.043 },
      ],
    });
    expect(f["throttle_nanos"]).toBe(0);
  });

  it("baselines on the snapshot then applies the ticked reprice, emitting rates events HEALTHY", () => {
    const { conn, sock, events } = makeConn();
    sock.open();
    const id = conn.subscribeRates(INSTR, CURVE, "5Y OIS");
    sock.deliver(serverSnapshotFrame(Number(id), 1, 1234.5));
    sock.deliver(serverUpdateFrame(Number(id), 2, 1300.25, 0.0001));

    const snaps = events.filter((e) => e.kind === "rates_snapshot");
    const upds = events.filter((e) => e.kind === "rates_update");
    expect(snaps.length).toBe(1);
    expect(upds.length).toBe(1);
    expect((snaps[0] as { snapshot: RatesStreamSnapshot }).snapshot.result.pv).toBeCloseTo(1234.5, 6);
    expect((upds[0] as { update: RatesStreamUpdate }).update.result.pv).toBeCloseTo(1300.25, 6);
    expect((upds[0] as { update: RatesStreamUpdate }).update.curveShift).toBeCloseTo(0.0001, 8);
    const healths = events.filter((e) => e.kind === "health").map((e) => (e as { health: string }).health);
    expect(healths).toContain("HEALTHY");
  });

  it("drops a stale-duplicate (not-newer) update — the rates sequence is contiguous by construction", () => {
    const { conn, sock, events } = makeConn();
    sock.open();
    const id = conn.subscribeRates(INSTR, CURVE, "5Y OIS");
    sock.deliver(serverSnapshotFrame(Number(id), 1, 1234.5));
    sock.deliver(serverUpdateFrame(Number(id), 2, 1300.25, 0.0001));
    // A replayed seq-2 straggler (e.g. post-reconnect) must NOT re-emit.
    sock.deliver(serverUpdateFrame(Number(id), 2, 9999.9, 0.0001));
    const upds = events.filter((e) => e.kind === "rates_update");
    expect(upds.length).toBe(1);
  });

  it("flips a silent rates line to STALE after the staleness window (never frozen-as-live)", () => {
    const { conn, sock, events, time } = makeConn();
    sock.open();
    const id = conn.subscribeRates(INSTR, CURVE, "5Y OIS");
    sock.deliver(serverSnapshotFrame(Number(id), 1, 1234.5));
    time.advance(1500); // exceed the 1000ms staleness window with no frame
    const healths = events.filter((e) => e.kind === "health").map((e) => (e as { health: string }).health);
    expect(healths).toContain("STALE");
  });

  it("unsubscribeRates sends the shared unsubscribe frame and stops the line", () => {
    const { conn, sock } = makeConn();
    sock.open();
    const id = conn.subscribeRates(INSTR, CURVE, "5Y OIS");
    conn.unsubscribeRates(id);
    const unsub = sock.sentOfType("unsubscribe");
    expect(unsub.length).toBe(1);
    expect(unsub[0]?.["subscription"]).toEqual({ value: Number(id) });
  });

  it("re-issues every live rates line on reconnect (fresh baseline, no rates resync)", () => {
    const { conn, sock, time } = makeConn();
    sock.open();
    conn.subscribeRates(INSTR, CURVE, "5Y OIS");
    expect(sock.sentOfType("rates_subscribe").length).toBe(1);
    // Drop: the connection schedules a backoff reconnect (via the injected timer).
    sock.close();
    // Advance past the backoff so the connection re-opens the (same fake) socket;
    // its onopen re-issues every live line with a fresh baseline.
    time.advance(1000);
    sock.open();
    // A second rates_subscribe is re-issued; NO `resync` is sent — the server has no
    // rates resync replay, so recovery is a fresh re-subscribe (baseline seq 1).
    expect(sock.sentOfType("rates_subscribe").length).toBe(2);
    expect(sock.sentOfType("resync").length).toBe(0);
  });
});

// ---------------------------------------------------------------------------
// function-layer shaping + cell render (parseRatesObservable / formatRatesSeriesCell)
// ---------------------------------------------------------------------------

describe("parseRatesObservable", () => {
  it("defaults to PV and maps canonical + short forms", () => {
    expect(parseRatesObservable(undefined)).toBe("PV");
    expect(parseRatesObservable("")).toBe("PV");
    expect(parseRatesObservable("pv")).toBe("PV");
    expect(parseRatesObservable("PAR")).toBe("PAR_RATE");
    expect(parseRatesObservable("par_rate")).toBe("PAR_RATE");
    expect(parseRatesObservable("rate")).toBe("PAR_RATE");
    expect(parseRatesObservable("PV01")).toBe("PV01");
    expect(parseRatesObservable("dv01")).toBe("DV01");
  });
  it("rejects an unknown observable", () => {
    expect(() => parseRatesObservable("gamma")).toThrow(ShapingError);
  });
});

describe("formatRatesSeriesCell", () => {
  const res = result(1234.5);
  it("renders each observable in its natural unit", () => {
    expect(formatRatesSeriesCell({ result: res, observable: "PV", health: "HEALTHY", baselined: true })).toBe(
      "1234.50",
    );
    expect(formatRatesSeriesCell({ result: res, observable: "PAR_RATE", health: "HEALTHY", baselined: true })).toBe(
      "4.1200%",
    );
    expect(formatRatesSeriesCell({ result: res, observable: "DV01", health: "HEALTHY", baselined: true })).toBe(
      "479.90",
    );
  });
  it("shows a waiting marker before a baseline / for a null result", () => {
    expect(formatRatesSeriesCell({ result: null, observable: "PV", health: "RESYNCING", baselined: false })).toBe(
      "… (awaiting)",
    );
  });
  it("dims a stale line and tags a resyncing one, never freezing-as-live", () => {
    expect(formatRatesSeriesCell({ result: res, observable: "PV", health: "STALE", baselined: true })).toBe(
      "… 1234.50 (stale)",
    );
    expect(formatRatesSeriesCell({ result: res, observable: "PV", health: "RESYNCING", baselined: true })).toBe(
      "1234.50 (resync)",
    );
  });
  it("keeps a small sub-1 sensitivity visible (6dp) rather than rounding it away", () => {
    const tiny: RatesPricingResult = { pv: 0, parRate: 0.04, pv01: 0.0005, dv01: 0.0005, keyRateLadder: [] };
    expect(formatRatesSeriesCell({ result: tiny, observable: "PV01", health: "HEALTHY", baselined: true })).toBe(
      "0.000500",
    );
  });
});
