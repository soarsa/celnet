/**
 * HedgeSeedContext — the one-shot HAND-OFF store that carries a "tailor the hedging
 * strategy for THIS deal's flow" request from a live Deals-blotter row to the Hedging →
 * Exit Policy rule builder, WITHOUT encoding the rule in the URL. The direct analogue of
 * {@link AcceptanceSeedContext}: a small, app-level shared store (mirroring `AppContext`)
 * holding a single pending {@link HedgeSeed} with a monotonic `nonce` so a consumer can
 * tell a fresh request from a stale one.
 *
 * Flow:
 *   1. A deal row's "Change hedging strategy" calls {@link requestHedgeSeed} with the
 *      deal's facts → sets `pending` with the next nonce.
 *   2. The Hedging workspace's Exit Policy tab watches `pending`, opens a NEW hedge-rule
 *      draft pre-scoped to that flow ({@link hedgeRuleFromSeed}), and shows a hint.
 *   3. The tab calls {@link consumeHedgeSeed} to clear it (the trader still picks the
 *      exit action + Saves manually — nothing auto-saves), so re-entering the tab does
 *      NOT re-seed.
 *
 * The default context value is a NO-OP (never `null`), so a component rendered WITHOUT
 * the provider — e.g. a standalone blotter in a unit test — never throws; it simply has
 * no cross-surface hand-off. This mirrors the resilience of the other app contexts.
 */
import { createContext, useCallback, useContext, useMemo, useRef, useState } from "react";

import type { HedgeSeedDeal } from "../lib/hedgeSeed";

/** A pending "seed a hedge rule for this deal's flow" request. */
export interface HedgeSeed {
  /** Monotonic id — lets a consumer distinguish a fresh request from a stale one. */
  nonce: number;
  /** The deal facts the seeded rule + hint are built from. */
  deal: HedgeSeedDeal;
}

/** The seed store surface: the pending request + the request/consume verbs. */
export interface HedgeSeedApi {
  /** The pending seed, or `null` when none is outstanding. */
  pending: HedgeSeed | null;
  /** Request a seed for `deal` (mints the next nonce; replaces any pending one). */
  requestHedgeSeed: (deal: HedgeSeedDeal) => void;
  /** Clear the pending seed once a consumer has applied it. */
  consumeHedgeSeed: () => void;
}

const NOOP_API: HedgeSeedApi = {
  pending: null,
  requestHedgeSeed: () => {},
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
    setPending({ nonce: nonceRef.current, deal });
  }, []);

  const consumeHedgeSeed = useCallback((): void => setPending(null), []);

  const value = useMemo<HedgeSeedApi>(
    () => ({ pending, requestHedgeSeed, consumeHedgeSeed }),
    [pending, requestHedgeSeed, consumeHedgeSeed],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

/** Read the hedge-seed store (a no-op store when no provider is mounted). */
export function useHedgeSeed(): HedgeSeedApi {
  return useContext(Ctx);
}
