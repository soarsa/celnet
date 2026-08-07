/**
 * AcceptanceSeedContext — the one-shot HAND-OFF store that carries a "seed an acceptance
 * rule for THIS counterparty" request from a live-flow row (the Deals / Quotes blotters)
 * to the Acceptance rule builder, WITHOUT encoding the rule in the URL. It is a small,
 * app-level shared store (mirroring the `AppContext` grammar): a single pending
 * {@link AcceptanceSeed} with a monotonic `nonce` so a consumer can tell a fresh request
 * from a stale one.
 *
 * Flow:
 *   1. A blotter row's "Create acceptance rule" calls {@link requestAcceptanceSeed} with
 *      the row's counterparty → sets `pending` with the next nonce.
 *   2. The consolidated Risk surface watches `pending` and switches to its Acceptance tab
 *      (revealing it read-only for a non-`manage_acceptance` holder rather than crashing).
 *   3. The AcceptanceWorkspace merges the seed into the CURRENT policy as an unsaved edit
 *      and calls {@link consumeAcceptanceSeed} to clear it (the user still Saves manually).
 *
 * The default context value is a NO-OP (never `null`), so a component rendered WITHOUT the
 * provider — e.g. a standalone blotter in a unit test — never throws; it simply has no
 * cross-surface hand-off. This mirrors the resilience of the other app contexts.
 */
import { createContext, useCallback, useContext, useMemo, useRef, useState } from "react";

/** A pending "seed an acceptance rule for this counterparty" request. */
export interface AcceptanceSeed {
  /** Monotonic id — lets a consumer distinguish a fresh request from a stale one. */
  nonce: number;
  /** The counterparty the seeded rule should match (`Counterparty = <this>`). */
  counterparty: string;
}

/** The seed store surface: the pending request + the request/consume verbs. */
export interface AcceptanceSeedApi {
  /** The pending seed, or `null` when none is outstanding. */
  pending: AcceptanceSeed | null;
  /** Request a seed for `counterparty` (mints the next nonce; replaces any pending one). */
  requestAcceptanceSeed: (counterparty: string) => void;
  /** Clear the pending seed once a consumer has applied it. */
  consumeAcceptanceSeed: () => void;
}

const NOOP_API: AcceptanceSeedApi = {
  pending: null,
  requestAcceptanceSeed: () => {},
  consumeAcceptanceSeed: () => {},
};

const Ctx = createContext<AcceptanceSeedApi>(NOOP_API);

export function AcceptanceSeedProvider({
  children,
}: {
  children: React.ReactNode;
}): React.ReactElement {
  const [pending, setPending] = useState<AcceptanceSeed | null>(null);
  const nonceRef = useRef(0);

  const requestAcceptanceSeed = useCallback((counterparty: string): void => {
    nonceRef.current += 1;
    setPending({ nonce: nonceRef.current, counterparty });
  }, []);

  const consumeAcceptanceSeed = useCallback((): void => setPending(null), []);

  const value = useMemo<AcceptanceSeedApi>(
    () => ({ pending, requestAcceptanceSeed, consumeAcceptanceSeed }),
    [pending, requestAcceptanceSeed, consumeAcceptanceSeed],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

/** Read the acceptance-seed store (a no-op store when no provider is mounted). */
export function useAcceptanceSeed(): AcceptanceSeedApi {
  return useContext(Ctx);
}
