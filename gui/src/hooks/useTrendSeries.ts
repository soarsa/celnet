/**
 * useTrendSeries — drives the blotter's non-PREMIUM TrendModes off the contract's
 * market-series feed (`MarketSeriesSubscribe`, served by celnet-server). When the
 * active mode is an observable (ATM_VOL / SPOT / RR / BF / FORWARD) this hook opens
 * one market-series subscription per visible blotter row (its pair + a
 * representative pillar tenor, plus the 25Δ wing for RR/BF), applies the opening
 * `MarketSeriesSnapshot` then each `MarketSeriesPoint`, and exposes a bounded
 * per-row value history the Sparkline plots — a REAL observed series, never a
 * fabricated line.
 *
 * It opens its OWN multiplexed StreamSession over the shared transport (the live
 * WS transport multiplexes it on the same connection; the mock ticks it
 * independently), so the price-blotter session in `useStreamSession` is untouched.
 * For PREMIUM (or the gated VEGA/PNL) it subscribes to nothing and returns empty —
 * the blotter falls back to the row's own `midHistory`.
 *
 * The series are conflated into ONE commit per animation frame (rAF batching),
 * matching the render-budget-as-latency-budget rule the price blotter uses.
 */

import { useEffect, useRef, useState } from "react";
import type { CcyPair, Tenor } from "../data/contract";
import type { CelnetTransport, StreamSession } from "../data/transport";
import {
  TREND_WING_DELTA,
  tenorForYears,
  trendModeSpec,
  type TrendMode,
} from "../lib/trend";

/** A bounded recent history for one row's trend series. */
export interface TrendSeries {
  /** The values, oldest → newest (bounded ring). */
  values: number[];
  /** The latest value (for a numeric readout), or undefined before any point. */
  latest?: number;
}

/** The blotter rows this hook needs to open a per-row series for. */
export interface TrendRowKey {
  /** The blotter subscription id — the key the result map is keyed on. */
  subscriptionId: bigint;
  pair: CcyPair;
  /** A representative tenor (year fraction) for tenor-dependent observables. */
  tenorYears: number;
}

const HISTORY_LEN = 48;

/**
 * Open market-series subscriptions for `mode` across `rows`, returning a map from
 * each row's blotter subscription id to its live trend series. Empty for PREMIUM
 * and the gated modes (the caller falls back to the row's own premium history).
 */
export function useTrendSeries(
  transport: CelnetTransport,
  mode: TrendMode,
  rows: TrendRowKey[],
): Map<bigint, TrendSeries> {
  const [series, setSeries] = useState<Map<bigint, TrendSeries>>(new Map());

  // The authoritative store lives in refs (mutated by the stream callback); React
  // state is committed once per frame from it.
  const byRow = useRef(new Map<bigint, TrendSeries>());
  // market-series subscription id → the blotter row id it backs.
  const seriesToRow = useRef(new Map<bigint, bigint>());
  const dirty = useRef(false);
  const raf = useRef(0);

  const spec = trendModeSpec(mode);
  const observable = spec.observable;
  // A stable signature of the row set so the effect re-subscribes only when the
  // visible rows (or their pillar) actually change — not every render.
  const rowSig = rows
    .map((r) => `${r.subscriptionId}:${r.pair.base}/${r.pair.quote}:${r.tenorYears.toFixed(6)}`)
    .join("|");

  useEffect(() => {
    // PREMIUM / gated modes stream nothing; clear any prior series.
    if (!observable) {
      byRow.current = new Map();
      seriesToRow.current = new Map();
      setSeries(new Map());
      return;
    }

    const session: StreamSession = transport.openStreamSession();
    const rowOf = seriesToRow.current;
    rowOf.clear();
    byRow.current = new Map();

    const scheduleCommit = (): void => {
      if (dirty.current) return;
      dirty.current = true;
      raf.current = requestAnimationFrame(() => {
        dirty.current = false;
        // Snapshot the store into a fresh Map so React sees a new reference.
        setSeries(new Map(byRow.current));
      });
    };

    const append = (rowId: bigint, value: number): void => {
      const prev = byRow.current.get(rowId);
      const values = prev ? prev.values.slice(-(HISTORY_LEN - 1)) : [];
      values.push(value);
      byRow.current.set(rowId, { values, latest: value });
    };

    const dispose = session.onEvent((event) => {
      switch (event.kind) {
        case "marketSeriesSnapshot": {
          const rowId = rowOf.get(event.snapshot.subscriptionId);
          if (rowId === undefined) break;
          // Seed the history from the snapshot's points (oldest → newest).
          const pts = event.snapshot.points.map((p) => p.value).slice(-HISTORY_LEN);
          const latest = pts.length > 0 ? pts[pts.length - 1] : undefined;
          byRow.current.set(
            rowId,
            latest === undefined ? { values: [] } : { values: pts, latest },
          );
          scheduleCommit();
          break;
        }
        case "marketSeriesPoint": {
          const rowId = rowOf.get(event.point.subscriptionId);
          if (rowId === undefined) break;
          append(rowId, event.point.value);
          scheduleCommit();
          break;
        }
        default:
          break;
      }
    });

    // Open one series per row. Tenor-dependent observables pin a representative
    // pillar tenor; SPOT is tenor-independent (no tenor). RR/BF carry the 25Δ wing.
    for (const row of rows) {
      const tenor: Tenor | undefined =
        observable === "SPOT" ? undefined : tenorForYears(row.tenorYears);
      const seriesId = session.subscribeMarketSeries({
        pair: row.pair,
        observable,
        ...(tenor ? { tenor } : {}),
        ...(spec.needsDelta ? { delta: TREND_WING_DELTA } : {}),
        throttleNanos: 0n,
        historyLimit: HISTORY_LEN,
      });
      rowOf.set(seriesId, row.subscriptionId);
    }

    return () => {
      dispose();
      cancelAnimationFrame(raf.current);
      session.close();
      rowOf.clear();
      byRow.current = new Map();
    };
    // Re-subscribe when the mode (observable) or the visible row set changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [transport, observable, spec.needsDelta, rowSig]);

  return series;
}
