/**
 * The market-series subscription registry: ref-counted, deduped, multiplexed.
 *
 * The mirror of `streamRegistry.ts` for the market-series (trend) feed. Office.js
 * coalesces streaming cells with equal arguments; identical-argument live trend
 * cells (`=CELNET.SERIES(...)`) SHARE ONE `market_series_subscribe` on the single
 * multiplexed session, ref-counting subscribers and tearing the server series
 * down only when the last cell is removed (`onCanceled`) — no orphans. It holds
 * only the latest observed value per series (a trend cell wants the freshest
 * point), seeded from the opening `series_snapshot`.
 *
 * It depends only on a minimal `SeriesConnection` seam (the subset of the real
 * Connection it needs), so it is unit-testable with an in-memory fake — no Office
 * host, no server, no mocks of our functionality (GUIDE.md guardrail #2).
 */

import type { MarketObservable } from "../contract/contract";
import type { MarketSeriesRequest, StreamEvent } from "../transport/connection";

/** The minimal connection surface the series registry drives. */
export interface SeriesConnection {
  subscribeSeries(req: MarketSeriesRequest): bigint;
  unsubscribeSeries(subscriptionId: bigint): void;
  onEvent(listener: (event: StreamEvent) => void): () => void;
}

/** A live observed value handed to a trend cell. */
export interface SeriesTick {
  /** The observable's latest value (vol for ATM_VOL/RR/BF; a rate for SPOT/FORWARD). */
  readonly value: number;
  /** The observable identity (echoed so a cell can label its unit). */
  readonly observable: MarketObservable;
  /** Observation time (ns since the Unix epoch); 0 until the first point lands. */
  readonly epochNanos: bigint;
  /** True once the opening snapshot has seeded a baseline value. */
  readonly baselined: boolean;
}

/** A single shared series: the server id, refcount, latest tick, subscribers. */
interface SharedSeries {
  readonly key: string;
  readonly serverId: bigint;
  readonly observable: MarketObservable;
  refcount: number;
  latest: SeriesTick;
  readonly sinks: Set<(tick: SeriesTick) => void>;
}

/**
 * A canonical, order-stable key for a series request so identical-argument cells
 * (regardless of typed-string whitespace/case, already normalized by the caller)
 * map to ONE shared server subscription.
 */
export function seriesKey(req: MarketSeriesRequest): string {
  return JSON.stringify([
    req.pair.base,
    req.pair.quote,
    req.observable,
    req.tenor ? [req.tenor.unit, req.tenor.count] : null,
    req.delta ?? null,
  ]);
}

/**
 * Ref-counts and multiplexes live trend cells onto shared server series over the
 * single connection. A cell calls `acquire(...)`; the returned `release()` is
 * what the streaming custom function's `onCanceled` invokes.
 */
export class SeriesRegistry {
  private readonly byKey = new Map<string, SharedSeries>();
  private readonly byServerId = new Map<bigint, SharedSeries>();
  private readonly detach: () => void;

  constructor(private readonly conn: SeriesConnection) {
    this.detach = this.conn.onEvent((e) => this.onEvent(e));
  }

  /**
   * Subscribe a cell to a live observable. Identical-argument cells share the
   * underlying server series (ref-counted). Returns the current tick and a
   * `release` for `onCanceled`.
   */
  acquire(
    req: MarketSeriesRequest,
    sink: (tick: SeriesTick) => void,
  ): { release: () => void; current: SeriesTick } {
    const key = seriesKey(req);
    let shared = this.byKey.get(key);
    if (!shared) {
      const serverId = this.conn.subscribeSeries(req);
      shared = {
        key,
        serverId,
        observable: req.observable,
        refcount: 0,
        latest: {
          value: Number.NaN,
          observable: req.observable,
          epochNanos: 0n,
          baselined: false,
        },
        sinks: new Set(),
      };
      this.byKey.set(key, shared);
      this.byServerId.set(serverId, shared);
    }
    shared.refcount += 1;
    shared.sinks.add(sink);
    sink(shared.latest);
    return {
      current: shared.latest,
      release: () => this.release(key, sink),
    };
  }

  private release(key: string, sink: (tick: SeriesTick) => void): void {
    const shared = this.byKey.get(key);
    if (!shared) return;
    shared.sinks.delete(sink);
    shared.refcount -= 1;
    if (shared.refcount <= 0) {
      this.conn.unsubscribeSeries(shared.serverId);
      this.byKey.delete(key);
      this.byServerId.delete(shared.serverId);
    }
  }

  /** Number of distinct shared series currently live (for assertions). */
  liveSeriesCount(): number {
    return this.byKey.size;
  }

  /** Total subscriber refcount across all shared series (for assertions). */
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
      case "series_snapshot": {
        const shared = this.byServerId.get(e.snapshot.subscriptionId);
        if (!shared) return;
        // Seed the baseline from the newest point in the opening history (oldest →
        // newest); an empty history baselines with the snapshot timestamp only.
        const last = e.snapshot.points[e.snapshot.points.length - 1];
        this.applyTick(shared, {
          value: last ? last.value : Number.NaN,
          observable: e.snapshot.observable,
          epochNanos: last ? last.epochNanos : e.snapshot.epochNanos,
          baselined: true,
        });
        break;
      }
      case "series_point": {
        const shared = this.byServerId.get(e.point.subscriptionId);
        if (!shared) return;
        // Latest-tick dedup: only ever hold/propagate the freshest value.
        this.applyTick(shared, {
          value: e.point.value,
          observable: shared.observable,
          epochNanos: e.point.epochNanos,
          baselined: true,
        });
        break;
      }
      default:
        break;
    }
  }

  private applyTick(shared: SharedSeries, tick: SeriesTick): void {
    shared.latest = tick;
    for (const sink of shared.sinks) sink(tick);
  }
}
