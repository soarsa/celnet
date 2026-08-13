/**
 * Owns the inbound-liquidity (LP) panel data for the Liquidity workspace.
 *
 * Polls `FixAdminService.ListLiquidityProviders` on a fixed cadence and derives
 * the one number the wire deliberately does NOT carry: a per-provider tick
 * **rate**. The server reports a monotonic lifetime `quoteUpdates` counter and
 * the instant it was read at; the rate is the delta of those two across
 * successive polls, which keeps the server free of per-client rate state and
 * survives a missed or slow poll (the divisor is the real elapsed time, not the
 * nominal interval).
 *
 * A selected provider additionally pulls its per-instrument drill-down in the
 * same round trip, so opening a row costs no extra request.
 */

import { useCallback, useEffect, useRef, useState } from "react";

import type { LiquidityPanel } from "../data/contract";
import type { CelnetTransport } from "../data/transport";

/** How often the panel re-reads the roster, in milliseconds. */
const POLL_INTERVAL_MS = 2000;
/** Nanoseconds per second — the wire carries epoch nanoseconds throughout. */
const NANOS_PER_SEC = 1e9;

/** Narrow an unknown thrown value to a display string. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : "unexpected error";
}

/** The empty panel — the pre-load state and the fallback on a failed poll. */
const EMPTY: LiquidityPanel = {
  providers: [],
  quotes: [],
  inboundEnabled: false,
  asOfNanos: 0,
};

/** The Liquidity-workspace data API. */
export interface LiquidityProvidersApi {
  /** The latest panel (empty until the first poll resolves). */
  panel: LiquidityPanel;
  /**
   * Quote-updates per second per provider, keyed by `connectionId` — derived
   * client-side by deltaing the server's lifetime counter across polls. A
   * provider is absent until it has been seen in two consecutive polls, since
   * one sample cannot establish a rate.
   */
  rates: Record<string, number>;
  /** True while the initial load is in flight (not on background refreshes). */
  isLoading: boolean;
  /** The last poll error as a display string, or `null` when healthy. */
  error: string | null;
  /** Force an immediate re-read. */
  refetch: () => Promise<void>;
}

export function useLiquidityProviders(
  transport: CelnetTransport,
  selected: string | null,
): LiquidityProvidersApi {
  const [panel, setPanel] = useState<LiquidityPanel>(EMPTY);
  const [rates, setRates] = useState<Record<string, number>>({});
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // The previous sample the rate is differenced against. A ref, not state: it is
  // an input to the next computation, never something a render reads.
  const previous = useRef<{
    asOfNanos: number;
    counts: Record<string, number>;
  } | null>(null);

  const load = useCallback(async (): Promise<void> => {
    try {
      const next = await transport.listLiquidityProviders(selected ?? undefined);
      const counts: Record<string, number> = {};
      for (const p of next.providers) counts[p.connectionId] = p.quoteUpdates;

      const prior = previous.current;
      const elapsedSecs = prior
        ? (next.asOfNanos - prior.asOfNanos) / NANOS_PER_SEC
        : 0;
      if (prior && elapsedSecs > 0) {
        const derived: Record<string, number> = {};
        for (const [id, count] of Object.entries(counts)) {
          const before = prior.counts[id];
          // A provider first seen in THIS poll has no baseline, and a counter
          // that went backwards means the edge restarted — neither yields an
          // honest rate, so both are simply omitted this round.
          if (before === undefined || count < before) continue;
          derived[id] = (count - before) / elapsedSecs;
        }
        setRates(derived);
      }
      previous.current = { asOfNanos: next.asOfNanos, counts };

      setPanel(next);
      setError(null);
    } catch (e: unknown) {
      setError(messageOf(e));
    } finally {
      setIsLoading(false);
    }
  }, [transport, selected]);

  useEffect(() => {
    let cancelled = false;
    const tick = (): void => {
      if (!cancelled) void load();
    };
    tick();
    const id = window.setInterval(tick, POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [load]);

  return { panel, rates, isLoading, error, refetch: load };
}
