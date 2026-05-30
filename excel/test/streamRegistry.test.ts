import { describe, expect, it } from "vitest";
import { StreamRegistry, type StreamConnection } from "../src/functions/streamRegistry";
import type { StreamEvent } from "../src/transport/connection";
import type { Conventions, Instrument, Snapshot, Update } from "../src/contract/contract";
import { DEFAULT_CONVENTIONS, shapeVanillaInstrument } from "../src/functions/shaping";

/** A scriptable in-memory connection: records subscribe/unsubscribe and lets a
 * test push server events. No server, no mock of OUR functionality — just the
 * transport seam the registry depends on. */
class FakeConnection implements StreamConnection {
  private nextId = 1n;
  readonly subscribed: bigint[] = [];
  readonly unsubscribed: bigint[] = [];
  private listener: ((e: StreamEvent) => void) | null = null;

  subscribe(_i: Instrument, _c: Conventions, _label: string): bigint {
    const id = this.nextId++;
    this.subscribed.push(id);
    return id;
  }
  unsubscribe(id: bigint): void {
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

function snapshot(subId: bigint, seq: bigint, bid: number, offer: number): Snapshot {
  return {
    subscriptionId: subId,
    sequence: seq,
    price: { bid, offer },
    greeks: zeroGreeks(),
    vol: 0.1,
    conventions: DEFAULT_CONVENTIONS,
    resolvedStrike: 1.12,
    tradable: [
      { token: 111n, side: "BUY", premium: offer, validUntilNanos: 0n },
      { token: 222n, side: "SELL", premium: bid, validUntilNanos: 0n },
    ],
    epochNanos: 0n,
  };
}

function update(subId: bigint, seq: bigint, bid: number, offer: number): Update {
  return {
    subscriptionId: subId,
    sequence: seq,
    price: { bid, offer },
    greeks: zeroGreeks(),
    vol: 0.1,
    tradable: [],
    epochNanos: 0n,
  };
}

function zeroGreeks() {
  return {
    price: 0, deltaSpot: 0, deltaForward: 0, gamma: 0, vega: 0, theta: 0,
    rhoDom: 0, rhoFor: 0, vanna: 0, volga: 0, charm: 0, speed: 0, zomma: 0, color: 0,
  };
}

const INSTR: Instrument = shapeVanillaInstrument({
  pair: "EURUSD", tenor: "1Y", strikeOrDelta: 1.12, callPut: "C", notional: 1e6,
});

describe("StreamRegistry coalescing + refcount", () => {
  it("shares one server subscription across identical-argument cells", () => {
    const conn = new FakeConnection();
    const reg = new StreamRegistry(conn);
    const a = reg.acquire(INSTR, DEFAULT_CONVENTIONS, "a", () => {});
    const b = reg.acquire(INSTR, DEFAULT_CONVENTIONS, "b", () => {});
    expect(conn.subscribed.length).toBe(1); // ONE server subscription for two cells
    expect(reg.liveSubscriptionCount()).toBe(1);
    expect(reg.totalRefcount()).toBe(2);

    // First release keeps the server subscription alive (still one subscriber).
    a.release();
    expect(conn.unsubscribed.length).toBe(0);
    expect(reg.totalRefcount()).toBe(1);
    // Last release tears it down (no orphaned server subscription).
    b.release();
    expect(conn.unsubscribed).toEqual(conn.subscribed);
    expect(reg.liveSubscriptionCount()).toBe(0);
  });

  it("opens distinct subscriptions for distinct structures", () => {
    const conn = new FakeConnection();
    const reg = new StreamRegistry(conn);
    const put = shapeVanillaInstrument({ pair: "EURUSD", tenor: "1Y", strikeOrDelta: 1.12, callPut: "P", notional: 1e6 });
    reg.acquire(INSTR, DEFAULT_CONVENTIONS, "c", () => {});
    reg.acquire(put, DEFAULT_CONVENTIONS, "p", () => {});
    expect(reg.liveSubscriptionCount()).toBe(2);
  });

  it("delivers snapshot then latest-tick updates to every shared cell", () => {
    const conn = new FakeConnection();
    const reg = new StreamRegistry(conn);
    const ticksA: { bid: number; health: string }[] = [];
    const ticksB: { bid: number; health: string }[] = [];
    reg.acquire(INSTR, DEFAULT_CONVENTIONS, "a", (t) => ticksA.push({ bid: t.price.bid, health: t.health }));
    reg.acquire(INSTR, DEFAULT_CONVENTIONS, "b", (t) => ticksB.push({ bid: t.price.bid, health: t.health }));
    const subId = conn.subscribed[0]!;

    conn.push({ kind: "snapshot", snapshot: snapshot(subId, 1n, 0.039, 0.041) });
    conn.push({ kind: "update", update: update(subId, 2n, 0.04, 0.042) });

    // Both cells saw the snapshot (HEALTHY) then the update.
    expect(ticksA.at(-1)).toEqual({ bid: 0.04, health: "HEALTHY" });
    expect(ticksB.at(-1)).toEqual({ bid: 0.04, health: "HEALTHY" });
  });

  it("propagates a STALE health to the cell holding its last-good value (never frozen-as-live)", () => {
    const conn = new FakeConnection();
    const reg = new StreamRegistry(conn);
    const ticks: { bid: number; health: string }[] = [];
    reg.acquire(INSTR, DEFAULT_CONVENTIONS, "a", (t) => ticks.push({ bid: t.price.bid, health: t.health }));
    const subId = conn.subscribed[0]!;
    conn.push({ kind: "snapshot", snapshot: snapshot(subId, 1n, 0.039, 0.041) });
    conn.push({ kind: "health", subscriptionId: subId, health: "STALE" });
    // Last tick keeps the last-good price but marks it stale.
    expect(ticks.at(-1)).toEqual({ bid: 0.039, health: "STALE" });
  });

  it("resolves a tradable handle to the latest BUY/SELL token for click-to-trade", () => {
    const conn = new FakeConnection();
    const reg = new StreamRegistry(conn);
    reg.acquire(INSTR, DEFAULT_CONVENTIONS, "a", () => {});
    const subId = conn.subscribed[0]!;
    conn.push({ kind: "snapshot", snapshot: snapshot(subId, 1n, 0.039, 0.041) });
    const buy = reg.resolveTradable(`sub:${subId}`, "BUY");
    expect(buy?.token).toBe(111n);
    expect(buy?.premium).toBe(0.041);
    const sell = reg.resolveTradable(`sub:${subId}`, "SELL");
    expect(sell?.token).toBe(222n);
  });
});
