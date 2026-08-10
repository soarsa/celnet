/**
 * HedgeSeedContext — the one-shot HAND-OFF store that carries a "seed a hedge exit-policy
 * rule for THIS source" request into the Hedging Rules → Exit Policy rule builder, WITHOUT
 * encoding the rule in the URL. The direct analogue of {@link AcceptanceSeedContext}: a
 * small, app-level shared store (mirroring `AppContext`) holding a single pending
 * {@link HedgeSeed} with a monotonic `nonce` so a consumer can tell a fresh request from
 * a stale one.
 *
 * TWO seed sources, one discriminated {@link HedgeSeedSource} union:
 *   • `deal`         — a Deals-blotter row's "Change hedging strategy" ({@link requestHedgeSeed});
 *   • `pricingGroup` — a pricing group's "Create hedging rule" (its editor button / roster
 *                      right-click, via {@link requestHedgeSeedFromPricingGroup}).
 *
 * Flow:
 *   1. A source calls its request verb → sets `pending` with the next nonce + the source.
 *   2. The Hedging workspace's Exit Policy tab watches `pending`, opens a NEW hedge-rule
 *      draft pre-scoped from that source, and shows a source-specific hint.
 *   3. The tab calls {@link consumeHedgeSeed} to clear it (the trader still picks the
 *      exit action + Saves manually — nothing auto-saves), so re-entering the tab does
 *      NOT re-seed.
 *
 * The default context value is a NO-OP (never `null`), so a component rendered WITHOUT
 * the provider — e.g. a standalone blotter in a unit test — never throws; it simply has
 * no cross-surface hand-off. This mirrors the resilience of the other app contexts.
 */
import { createContext, useCallback, useContext, useMemo, useRef, useState } from "react";

import type { HedgeSeedDeal, HedgeSeedPricingGroup } from "../lib/hedgeSeed";

/** The discriminated origin of a pending hedge seed — a booked deal or a pricing group. */
export type HedgeSeedSource =
  | { kind: "deal"; deal: HedgeSeedDeal }
  | { kind: "pricingGroup"; group: HedgeSeedPricingGroup };

/** A pending "seed a hedge rule from this source" request. */
export interface HedgeSeed {
  /** Monotonic id — lets a consumer distinguish a fresh request from a stale one. */
  nonce: number;
  /** The source the seeded rule + hint are built from (deal or pricing group). */
  source: HedgeSeedSource;
}

/** The seed store surface: the pending request + the request/consume verbs. */
export interface HedgeSeedApi {
  /** The pending seed, or `null` when none is outstanding. */
  pending: HedgeSeed | null;
  /** Request a seed for `deal` (mints the next nonce; replaces any pending one). */
  requestHedgeSeed: (deal: HedgeSeedDeal) => void;
  /** Request a seed for a pricing `group` (mints the next nonce; replaces any pending one). */
  requestHedgeSeedFromPricingGroup: (group: HedgeSeedPricingGroup) => void;
  /** Clear the pending seed once a consumer has applied it. */
  consumeHedgeSeed: () => void;
}

const NOOP_API: HedgeSeedApi = {
  pending: null,
  requestHedgeSeed: () => {},
  requestHedgeSeedFromPricingGroup: () => {},
  consumeHedgeSeed: () => {},
};

const Ctx = createContext<HedgeSeedApi>(NOOP_API);

export function HedgeSeedProvider({
  children,
}: {
  children: React.ReactNode;
}): React.ReactElement {
  const [pending, setPending] = useState<HedgeSeed | null>(null);
  const nonceRef = useRef(0);

  const requestHedgeSeed = useCallback((deal: HedgeSeedDeal): void => {
    nonceRef.current += 1;
    setPending({ nonce: nonceRef.current, source: { kind: "deal", deal } });
  }, []);

  const requestHedgeSeedFromPricingGroup = useCallback((group: HedgeSeedPricingGroup): void => {
    nonceRef.current += 1;
    setPending({ nonce: nonceRef.current, source: { kind: "pricingGroup", group } });
  }, []);

  const consumeHedgeSeed = useCallback((): void => setPending(null), []);

  const value = useMemo<HedgeSeedApi>(
    () => ({ pending, requestHedgeSeed, requestHedgeSeedFromPricingGroup, consumeHedgeSeed }),
    [pending, requestHedgeSeed, requestHedgeSeedFromPricingGroup, consumeHedgeSeed],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

/** Read the hedge-seed store (a no-op store when no provider is mounted). */
export function useHedgeSeed(): HedgeSeedApi {
  return useContext(Ctx);
}
