/**
 * The streaming subscription registry: ref-counted, deduped, multiplexed.
 *
 * Office.js coalesces streaming cells with equal arguments, and the design
 * (docs/EXCEL-INTEGRATION.md §3.2) requires that identical-argument live cells
 * SHARE ONE subscription on the single multiplexed `StreamSession`, ref-counting
 * subscribers and tearing the server subscription down only when the last cell is
 * removed (`onCanceled`). This registry implements exactly that, plus the
 * latest-tick dedup (Excel only needs the freshest value per cell) and the
 * stale/health propagation.
 *
 * It depends only on a minimal `StreamConnection` seam (the subset of the real
 * Connection it needs), so it is unit-testable with an in-memory fake — no Office
 * host, no server, no mocks of our functionality.
 */

import type { Conventions, Instrument, StreamHealth, TwoWayPrice } from "../contract/contract";
import type { StreamEvent } from "../transport/connection";
import { subscriptionKey } from "./shaping";

/** The minimal connection surface the registry drives (the real Connection fits). */
export interface StreamConnection {
  subscribe(instrument: Instrument, conventions: Conventions, label: string): bigint;
  unsubscribe(subscriptionId: bigint): void;
  onEvent(listener: (event: StreamEvent) => void): () => void;
}

/** A live streamed value handed to a cell (price + health + a tradable handle). */
export interface LiveTick {
  readonly price: TwoWayPrice;
  readonly vol: number;
  readonly health: StreamHealth;
  /**
   * An opaque handle to the latest BUY/SELL tradable tokens for click-to-trade
   * (the task-pane Trade button resolves it). NOT the raw token — the design
   * keeps the raw token off the grid; the handle indexes the registry's token
   * table for the cell's subscription.
   */
  readonly tradableHandle: string;
}

/** A single shared subscription: the server id, its refcount, and latest tick. */
interface SharedSub {
  readonly key: string;
  readonly serverId: bigint;
  refcount: number;
  latest: LiveTick;
  /** The per-subscriber callbacks (one per live cell sharing this subscription). */
  readonly sinks: Set<(tick: LiveTick) => void>;
  /** The latest tradable tokens, indexed by `tradableHandle` for click-to-trade. */
  tokens: { token: bigint; side: "BUY" | "SELL"; premium: number; validUntilNanos: bigint }[];
}

const INITIAL_TICK: LiveTick = {
  price: { bid: 0, offer: 0 },
  vol: 0,
  health: "RESYNCING",
  tradableHandle: "",
};

/**
 * Ref-counts and multiplexes live cells onto shared server subscriptions over the
 * single connection. A cell calls `acquire(...)`; the returned `release()` is
 * what the custom function's `onCanceled` invokes.
 */
export class StreamRegistry {
  private readonly byKey = new Map<string, SharedSub>();
  private readonly byServerId = new Map<bigint, SharedSub>();
  private readonly detach: () => void;

  constructor(private readonly conn: StreamConnection) {
    this.detach = this.conn.onEvent((e) => this.onEvent(e));
  }

  /**
   * Subscribe a cell to a live instrument. Identical-argument cells share the
   * underlying subscription (ref-counted). Returns the current tick (so the cell
   * shows the last-good value immediately) and a `release` for `onCanceled`.
   */
  acquire(
    instrument: Instrument,
    conventions: Conventions,
    label: string,
    sink: (tick: LiveTick) => void,
  ): { release: () => void; current: LiveTick } {
    const key = subscriptionKey(instrument, conventions);
    let shared = this.byKey.get(key);
    if (!shared) {
      const serverId = this.conn.subscribe(instrument, conventions, label);
      shared = {
        key,
        serverId,
        refcount: 0,
        latest: INITIAL_TICK,
        sinks: new Set(),
        tokens: [],
      };
      this.byKey.set(key, shared);
      this.byServerId.set(serverId, shared);
    }
    shared.refcount += 1;
    shared.sinks.add(sink);
    // Push the current value immediately so the cell is not blank until the next tick.
    sink(shared.latest);
    return {
      current: shared.latest,
      release: () => this.release(key, sink),
    };
  }

  private release(key: string, sink: (tick: LiveTick) => void): void {
    const shared = this.byKey.get(key);
    if (!shared) return;
    shared.sinks.delete(sink);
    shared.refcount -= 1;
    if (shared.refcount <= 0) {
      // Last subscriber gone: tear the server subscription down (no orphans).
      this.conn.unsubscribe(shared.serverId);
      this.byKey.delete(key);
      this.byServerId.delete(shared.serverId);
    }
  }

  /** Resolve a cell's `tradableHandle` to the latest matching tradable token. */
  resolveTradable(
    handle: string,
    side: "BUY" | "SELL",
  ): { serverId: bigint; token: bigint; premium: number; validUntilNanos: bigint } | undefined {
    const shared = this.byServerIdForHandle(handle);
    if (!shared) return undefined;
    const tok = shared.tokens.find((t) => t.side === side);
    if (!tok) return undefined;
    return {
      serverId: shared.serverId,
      token: tok.token,
      premium: tok.premium,
      validUntilNanos: tok.validUntilNanos,
    };
  }

  private byServerIdForHandle(handle: string): SharedSub | undefined {
    // The handle is `sub:<serverId>`; recover the subscription it indexes.
    const m = /^sub:(\d+)$/.exec(handle);
    if (!m) return undefined;
    return this.byServerId.get(BigInt(m[1] as string));
  }

  /** Number of distinct shared subscriptions currently live (for assertions). */
  liveSubscriptionCount(): number {
    return this.byKey.size;
  }

  /** Total subscriber refcount across all shared subscriptions (for assertions). */
  totalRefcount(): number {
    let total = 0;
    for (const s of this.byKey.values()) total += s.refcount;
    return total;
  }

  /** Dispose the registry (detach the connection listener). */
  dispose(): void {
    this.detach();
    this.byKey.clear();
    this.byServerId.clear();
  }

  private onEvent(e: StreamEvent): void {
    switch (e.kind) {
      case "snapshot": {
        const shared = this.byServerId.get(e.snapshot.subscriptionId);
        if (!shared) return;
        shared.tokens = e.snapshot.tradable.map((t) => ({
          token: t.token,
          side: t.side === "SELL" ? "SELL" : "BUY",
          premium: t.premium,
          validUntilNanos: t.validUntilNanos,
        }));
        this.applyTick(shared, {
          price: e.snapshot.price,
          vol: e.snapshot.vol,
          health: "HEALTHY",
          tradableHandle: `sub:${shared.serverId}`,
        });
        break;
      }
      case "update": {
        const shared = this.byServerId.get(e.update.subscriptionId);
        if (!shared) return;
        if (e.update.tradable.length > 0) {
          shared.tokens = e.update.tradable.map((t) => ({
            token: t.token,
            side: t.side === "SELL" ? "SELL" : "BUY",
            premium: t.premium,
            validUntilNanos: t.validUntilNanos,
          }));
        }
        // Latest-tick dedup: the registry only ever holds/propagates the freshest
        // value, so a slow cell never replays a backlog — it sees the newest tick.
        this.applyTick(shared, {
          price: e.update.price,
          vol: e.update.vol,
          health: shared.latest.health === "STALE" ? "HEALTHY" : shared.latest.health,
          tradableHandle: `sub:${shared.serverId}`,
        });
        break;
      }
      case "health": {
        const shared = this.byServerId.get(e.subscriptionId);
        if (!shared) return;
        // Re-emit the latest value under the new health (stale/resyncing/healthy)
        // so the cell shows the last-good number in a dimmed/stale state rather
        // than a frozen-as-live number or an #N/A storm (docs §5).
        this.applyTick(shared, { ...shared.latest, health: e.health });
        break;
      }
      default:
        break;
    }
  }

  private applyTick(shared: SharedSub, tick: LiveTick): void {
    shared.latest = tick;
    for (const sink of shared.sinks) sink(tick);
  }
}
