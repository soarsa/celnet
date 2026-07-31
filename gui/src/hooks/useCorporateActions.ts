/**
 * useCorporateActions — the data hook backing the Corporate Actions workspace.
 *
 * A thin layer over the transport's `CorporateActionsService` surface plus the
 * `AuthService` instrument registry (for the ISIN → instrument-id join the schedule
 * read needs). It loads the CA inbox and the instrument registry when `enabled`
 * (any authenticated session — the list reads sit on the `view` floor), and exposes
 * the confirm / apply lifecycle drivers (server-gated on the `refdata` capability).
 * It re-fetches the inbox after each mutation so the table reflects the server's
 * authoritative state; confirm / apply reject so the caller can surface a per-action
 * failure. When `enabled` is false (anonymous) it stays idle with an empty inbox.
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import type {
  ApplyCorporateActionResponse,
  CorporateAction,
  InstrumentDef,
  ListInstrumentScheduleResponse,
} from "../data/contract";
import type { CelnetTransport } from "../data/transport";

/** Narrow an unknown thrown value to a display string. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : "unexpected error";
}

/** The Corporate Actions workspace data API. */
export interface CorporateActionsApi {
  /** The current CA inbox (empty until the first load resolves). */
  actions: CorporateAction[];
  /** The instrument registry keyed by ISIN — the CA → schedule join + display names. */
  instrumentByIsin: Map<string, InstrumentDef>;
  /** True while a load (or refetch) is in flight. */
  isLoading: boolean;
  /** The last load error as a display string, or `null`. */
  error: string | null;
  /** Re-load the CA inbox (and the instrument registry) from the server. */
  refetch: () => Promise<void>;
  /** Resolve the effective post-CA cashflow schedule for one instrument id. */
  loadSchedule: (instrumentId: string) => Promise<ListInstrumentScheduleResponse>;
  /** Confirm a CA (`announced|elected → confirmed`); resolves the new version or rejects. */
  confirm: (caId: string) => Promise<CorporateAction>;
  /** Apply a confirmed CA against `heldFace`; resolves the apply result or rejects. */
  apply: (caId: string, heldFace: number) => Promise<ApplyCorporateActionResponse>;
}

export function useCorporateActions(
  transport: CelnetTransport,
  enabled: boolean,
): CorporateActionsApi {
  const [actions, setActions] = useState<CorporateAction[]>([]);
  const [instruments, setInstruments] = useState<InstrumentDef[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refetch = useCallback(async (): Promise<void> => {
    if (!enabled) {
      setActions([]);
      setInstruments([]);
      setError(null);
      return;
    }
    setIsLoading(true);
    try {
      // Independent reads — fetch in parallel (no request waterfall).
      const [inbox, defs] = await Promise.all([
        transport.listCorporateActions({}),
        transport.listInstruments(),
      ]);
      setActions(inbox.actions);
      setInstruments(defs);
      setError(null);
    } catch (e: unknown) {
      setError(messageOf(e));
    } finally {
      setIsLoading(false);
    }
  }, [transport, enabled]);

  useEffect(() => {
    void refetch();
  }, [refetch]);

  const instrumentByIsin = useMemo(() => {
    const map = new Map<string, InstrumentDef>();
    for (const def of instruments) {
      for (const id of def.externalIds) {
        if (id.scheme === "isin") map.set(id.value, def);
      }
    }
    return map;
  }, [instruments]);

  const loadSchedule = useCallback(
    (instrumentId: string): Promise<ListInstrumentScheduleResponse> =>
      transport.listInstrumentSchedule({ instrumentId }),
    [transport],
  );

  const confirm = useCallback(
    async (caId: string): Promise<CorporateAction> => {
      const res = await transport.confirmCorporateAction({ caId });
      await refetch();
      return res.action;
    },
    [transport, refetch],
  );

  const apply = useCallback(
    async (caId: string, heldFace: number): Promise<ApplyCorporateActionResponse> => {
      const res = await transport.applyCorporateAction({ caId, heldFace });
      await refetch();
      return res;
    },
    [transport, refetch],
  );

  return { actions, instrumentByIsin, isLoading, error, refetch, loadSchedule, confirm, apply };
}
