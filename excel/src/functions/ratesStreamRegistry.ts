/**
 * The fixed-income (linear-rates) streaming registry: ref-counted, deduped,
 * multiplexed — the FI counterpart of `streamRegistry.ts` (FX price streams) and
 * `seriesRegistry.ts` (market-series trends), folded onto the SAME multiplexed
 * session (the `rates-stream-ws` wire, commit 5ea82d6).
 *
 * Office.js coalesces streaming cells with equal arguments; identical-argument live
 * rates cells (`=CELNET.RATESSERIES(...)`) SHARE ONE `rates_subscribe` on the
 * single multiplexed session, ref-counting subscribers and tearing the server line
 * down only when the last cell is removed (`onCanceled`) — no orphans. Crucially,
 * the shared key is (instrument, curve) ONLY — NOT the displayed observable — so a
 * PV cell, a PV01 cell and a DV01 cell on the SAME instrument+curve all share ONE
 * server subscription: the streamed `RatesPricingResult` carries every measure
 * (pv / par_rate / pv01 / dv01 + the key-rate ladder) at once, and each cell just
 * PROJECTS its field client-side. It holds only the latest tick per line (a live
 * cell wants the freshest re-price), seeded from the opening snapshot, and
 * propagates the stale/health transition so a frozen line is never shown as live.
 *
 * A rates stream is INDICATIVE — it carries no click-to-trade token (rates
 * click-to-trade books through the RFQ/desk path), so there is no token table here
 * (the sole structural difference from `streamRegistry.ts`).
 *
 * It depends only on a minimal `RatesStreamConnection` seam (the subset of the real
 * Connection it needs), so it is unit-testable with an in-memory fake — no Office
 * host, no server, no mocks of our functionality (CLAUDE.md guardrail #2).
 */

import type { RatesCurveSet, RatesInstrument, RatesPricingResult, StreamHealth } from "../contract/contract";
import type { StreamEvent } from "../transport/connection";

/** The minimal connection surface the rates registry drives (the real Connection fits). */
export interface RatesStreamConnection {
  subscribeRates(
    instrument: RatesInstrument,
    curve: RatesCurveSet,
    label: string,
    throttleNanos?: bigint,
  ): bigint;
  unsubscribeRates(subscriptionId: bigint): void;
  onEvent(listener: (event: StreamEvent) => void): () => void;
}

/** A live streamed rates value handed to a cell (the whole priced result + health). */
export interface RatesLiveTick {
  /** The latest priced PV + par / PV01 / DV01 / key-rate ladder; `null` until baselined. */
  readonly result: RatesPricingResult | null;
  /** The parallel curve shift (decimal) this re-price was taken at (0 at baseline). */
  readonly curveShift: number;
  /** The line health (HEALTHY / RESYNCING / STALE). */
  readonly health: StreamHealth;
  /** True once the opening snapshot has seeded a baseline result. */
  readonly baselined: boolean;
  /** Observation time (ns since the Unix epoch); 0 until the first frame lands. */
  readonly epochNanos: bigint;
}

/** A single shared rates line: the server id, refcount, latest tick, subscribers. */
interface SharedRatesSub {
  readonly key: string;
  readonly serverId: bigint;
  refcount: number;
  latest: RatesLiveTick;
  readonly sinks: Set<(tick: RatesLiveTick) => void>;
}

const INITIAL_TICK: RatesLiveTick = {
  result: null,
  curveShift: 0,
  health: "RESYNCING",
  baselined: false,
  epochNanos: 0n,
};

/**
 * A canonical, order-stable key for a rates line so identical-argument cells map to
 * ONE shared server subscription. Fields are extracted explicitly (not raw
 * `JSON.stringify` of the structs) so object key order can never fragment the key.
 * The displayed observable is deliberately NOT part of the key — see the module doc.
 */
export function ratesKey(instrument: RatesInstrument, curve: RatesCurveSet): string {
  const curveKey = [
    curve.currency,
    curve.referenceDate.year,
    curve.referenceDate.month,
    curve.referenceDate.day,
    curve.pillars.map((p) => [p.tenorYears, p.parRate]),
  ];
  return JSON.stringify([instrumentKey(instrument), curveKey]);
}

/** The canonical projection of one rates instrument arm (the discriminant + its terms). */
function instrumentKey(instrument: RatesInstrument): unknown {
  switch (instrument.kind) {
    case "ois": {
      const i = instrument.ois;
      return ["ois", i.tenorYears, i.fixedRate, i.notional, i.direction];
    }
    case "irs": {
      const i = instrument.irs;
      return [
        "irs",
        i.tenorYears,
        i.fixedRate,
        i.notional,
        i.direction,
        i.fixedFrequency,
        i.fixedDayCount,
        i.floatFrequency,
        i.floatDayCount,
      ];
    }
    case "fra": {
      const i = instrument.fra;
      return ["fra", i.startMonths, i.endMonths, i.fixedRate, i.notional, i.direction, i.accrualBasis];
    }
    case "bond": {
      const i = instrument.bond;
      return [
        "bond",
        i.couponRate,
        i.couponFrequency,
        i.dayCount,
        i.maturityDate.year,
        i.maturityDate.month,
        i.maturityDate.day,
        i.redemption,
        i.side,
      ];
    }
  }
}

/**
 * Ref-counts and multiplexes live rates cells onto shared server subscriptions over
 * the single connection. A cell calls `acquire(...)`; the returned `release()` is
 * what the streaming custom function's `onCanceled` invokes.
 */
export class RatesStreamRegistry {
  private readonly byKey = new Map<string, SharedRatesSub>();
  private readonly byServerId = new Map<bigint, SharedRatesSub>();
  private readonly detach: () => void;

  constructor(private readonly conn: RatesStreamConnection) {
    this.detach = this.conn.onEvent((e) => this.onEvent(e));
  }

  /**
   * Subscribe a cell to a live rates line. Identical-argument cells share the
   * underlying server subscription (ref-counted). Returns the current tick (so the
   * cell shows the last-good value immediately) and a `release` for `onCanceled`.
   */
  acquire(
    instrument: RatesInstrument,
    curve: RatesCurveSet,
    label: string,
    sink: (tick: RatesLiveTick) => void,
    throttleNanos?: bigint,
  ): { release: () => void; current: RatesLiveTick } {
    const key = ratesKey(instrument, curve);
    let shared = this.byKey.get(key);
    if (!shared) {
      const serverId = this.conn.subscribeRates(instrument, curve, label, throttleNanos);
      shared = {
        key,
        serverId,
        refcount: 0,
        latest: INITIAL_TICK,
        sinks: new Set(),
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

  private release(key: string, sink: (tick: RatesLiveTick) => void): void {
    const shared = this.byKey.get(key);
    if (!shared) return;
    shared.sinks.delete(sink);
    shared.refcount -= 1;
    if (shared.refcount <= 0) {
      // Last subscriber gone: tear the server line down (no orphans).
      this.conn.unsubscribeRates(shared.serverId);
      this.byKey.delete(key);
      this.byServerId.delete(shared.serverId);
    }
  }

  /** Number of distinct shared rates lines currently live (for assertions). */
  liveSubscriptionCount(): number {
    return this.byKey.size;
  }

  /** Total subscriber refcount across all shared lines (for assertions). */
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
      case "rates_snapshot": {
        const shared = this.byServerId.get(e.snapshot.subscriptionId);
        if (!shared) return;
        this.applyTick(shared, {
          result: e.snapshot.result,
          curveShift: e.snapshot.curveShift,
          health: "HEALTHY",
          baselined: true,
          epochNanos: e.snapshot.epochNanos,
        });
        break;
      }
      case "rates_update": {
        const shared = this.byServerId.get(e.update.subscriptionId);
        if (!shared) return;
        // Latest-tick dedup: hold/propagate only the freshest re-price, so a slow
        // cell never replays a backlog — it always sees the newest PV/risk.
        this.applyTick(shared, {
          result: e.update.result,
          curveShift: e.update.curveShift,
          health: shared.latest.health === "STALE" ? "HEALTHY" : shared.latest.health,
          baselined: true,
          epochNanos: e.update.epochNanos,
        });
        break;
      }
      case "health": {
        const shared = this.byServerId.get(e.subscriptionId);
        if (!shared) return;
        // Re-emit the last-good value under the new health so the cell shows the
        // last number in a dimmed/stale state rather than freezing-as-live.
        this.applyTick(shared, { ...shared.latest, health: e.health });
        break;
      }
      default:
        break;
    }
  }

  private applyTick(shared: SharedRatesSub, tick: RatesLiveTick): void {
    shared.latest = tick;
    for (const sink of shared.sinks) sink(tick);
  }
}
