import { describe, expect, it } from "vitest";
import { Connection, type StreamEvent } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import { DEFAULT_CONVENTIONS, shapeVanillaInstrument } from "../src/functions/shaping";

/** A controllable in-memory socket: capture sent frames, drive inbound frames,
 * and open/close at will. The mirror is text JSON only, so it is all strings. */
class FakeSocket implements WebSocketLike {
  readyState = 0; // CONNECTING
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
    this.readyState = 1; // OPEN
    this.onopen?.();
  }
  deliver(frame: Record<string, unknown>): void {
    this.onmessage?.(JSON.stringify(frame));
  }
  sentOfType(type: string): Record<string, unknown>[] {
    return this.sent.filter((f) => f["type"] === type);
  }
}

/** A manual clock + timer scheduler so staleness/backoff are deterministic. */
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
  /** Advance the clock, firing any timers whose deadline has passed (once). */
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

const INSTR = shapeVanillaInstrument({
  pair: "EURUSD", tenor: "1Y", strikeOrDelta: 1.12, callPut: "C", notional: 1e6,
});

function snapshotFrame(subId: number, seq: number): Record<string, unknown> {
  return {
    type: "snapshot",
    subscription: { value: subId },
    sequence: seq,
    price: { bid: 0.039, offer: 0.041 },
    greeks: {},
    vol: 0.1,
    conventions: {},
    resolved_strike: 1.12,
    tradable: [{ token: 1, side: 0, premium: 0.041, valid_until_nanos: 0 }],
    epoch_nanos: 0,
  };
}
function updateFrame(subId: number, seq: number): Record<string, unknown> {
  return {
    type: "update",
    subscription: { value: subId },
    sequence: seq,
    price: { bid: 0.04, offer: 0.042 },
    greeks: {},
    vol: 0.1,
    tradable: [],
    epoch_nanos: 0,
  };
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

describe("Connection RFS lifecycle", () => {
  it("subscribes, baselines on snapshot, and applies in-sequence updates as HEALTHY", () => {
    const { conn, sock, events } = makeConn();
    sock.open();
    const subId = conn.subscribe(INSTR, DEFAULT_CONVENTIONS, "row");
    expect(sock.sentOfType("subscribe").length).toBe(1);

    sock.deliver(snapshotFrame(Number(subId), 1));
    sock.deliver(updateFrame(Number(subId), 2));

    const healths = events.filter((e) => e.kind === "health").map((e) => (e as { health: string }).health);
    expect(healths).toContain("HEALTHY");
    const updates = events.filter((e) => e.kind === "update");
    expect(updates.length).toBe(1);
    // No resync sent on an in-sequence stream.
    expect(sock.sentOfType("resync").length).toBe(0);
  });

  it("detects a sequence gap and sends a server-assisted resync", () => {
    const { conn, sock, events } = makeConn();
    sock.open();
    const subId = conn.subscribe(INSTR, DEFAULT_CONVENTIONS, "row");
    sock.deliver(snapshotFrame(Number(subId), 1));
    // Jump from seq 1 to seq 4 (missed 2,3): a gap.
    sock.deliver(updateFrame(Number(subId), 4));
    expect(sock.sentOfType("resync").length).toBe(1);
    expect(sock.sentOfType("resync")[0]?.["last_sequence"]).toBe(1);
    const healths = events.filter((e) => e.kind === "health").map((e) => (e as { health: string }).health);
    expect(healths).toContain("RESYNCING");
  });

  it("flips a silent subscription to STALE after the staleness window (never frozen-as-live)", () => {
    const { conn, sock, time, events } = makeConn();
    sock.open();
    const subId = conn.subscribe(INSTR, DEFAULT_CONVENTIONS, "row");
    sock.deliver(snapshotFrame(Number(subId), 1));
    // No further frames; advance past the staleness window so the monitor fires.
    time.advance(1500);
    const last = events.filter((e) => e.kind === "health").at(-1) as { health: string };
    expect(last.health).toBe("STALE");
  });

  it("a heartbeat resets liveness so a quiet-but-alive stream stays HEALTHY", () => {
    const { conn, sock, time, events } = makeConn();
    sock.open();
    const subId = conn.subscribe(INSTR, DEFAULT_CONVENTIONS, "row");
    sock.deliver(snapshotFrame(Number(subId), 1));
    time.advance(800);
    sock.deliver({ type: "heartbeat", subscription: { value: Number(subId) }, sequence: 1, epoch_nanos: 0 });
    time.advance(800); // total 1600 since snapshot, but only 800 since heartbeat
    const last = events.filter((e) => e.kind === "health").at(-1) as { health: string };
    expect(last.health).not.toBe("STALE");
  });

  it("a heartbeat ahead of our sequence triggers a resync (silent gap detected)", () => {
    const { conn, sock } = makeConn();
    sock.open();
    const subId = conn.subscribe(INSTR, DEFAULT_CONVENTIONS, "row");
    sock.deliver(snapshotFrame(Number(subId), 1));
    sock.deliver({ type: "heartbeat", subscription: { value: Number(subId) }, sequence: 5, epoch_nanos: 0 });
    expect(sock.sentOfType("resync").length).toBe(1);
  });

  it("on reconnect re-subscribes and resyncs every live line from its last good sequence", () => {
    const { conn, sock, time } = makeConn();
    sock.open();
    const subId = conn.subscribe(INSTR, DEFAULT_CONVENTIONS, "row");
    sock.deliver(snapshotFrame(Number(subId), 1));
    sock.deliver(updateFrame(Number(subId), 2));
    // Drop: the connection schedules a backoff reconnect (via the injected timer).
    sock.close();
    // Advance past the backoff so the connection re-opens the (same fake) socket;
    // the internal open() re-wires this socket's handlers, so the next open() fires
    // the connection's onopen → re-subscribe + resync.
    time.advance(1000);
    sock.open();
    // A fresh subscribe + a resync from the last good sequence (2) is re-issued.
    expect(sock.sentOfType("subscribe").length).toBe(2);
    const resyncs = sock.sentOfType("resync");
    expect(resyncs.at(-1)?.["last_sequence"]).toBe(2);
  });
});

describe("Connection request/response", () => {
  it("routes a reply to its correlation-id waiter and resolves the promise", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.requestQuote(INSTR, DEFAULT_CONVENTIONS, "k");
    const sent = sock.sentOfType("request_quote")[0]!;
    const corr = sent["correlation_id"] as number;
    sock.deliver({
      type: "quote",
      correlation_id: corr,
      quote_id: 7,
      idempotency_key: "k",
      price: { bid: 0.039, offer: 0.041 },
      greeks: {},
      conventions: {},
      resolved_strike: 1.12,
      epoch_nanos: 0,
      valid_until_nanos: 0,
    });
    const quote = await p;
    expect(quote.quoteId).toBe(7n);
    expect(quote.price.offer).toBe(0.041);
  });

  it("fails outstanding waiters fast on a drop (never an infinite await)", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.requestQuote(INSTR, DEFAULT_CONVENTIONS, "k");
    sock.close();
    await expect(p).rejects.toThrow(/closed/);
  });

  it("rejects on a typed error frame", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.requestQuote(INSTR, DEFAULT_CONVENTIONS, "k");
    const corr = sock.sentOfType("request_quote")[0]!["correlation_id"] as number;
    sock.deliver({ type: "error", correlation_id: corr, message: "failed_precondition: unknown surface_version" });
    await expect(p).rejects.toThrow(/failed_precondition/);
  });
});
