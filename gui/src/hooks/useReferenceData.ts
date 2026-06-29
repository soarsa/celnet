/**
 * useReferenceData — the data hook backing the Reference Data workspace's
 * instrument-definition registry.
 *
 * A thin layer over the transport's `AuthService` instrument surface: it loads
 * every instrument definition when `enabled` (any authenticated session — the
 * list/get RPCs are open to all signed-in users), and exposes the mutating
 * operations (create/update/delete) which the server gates to administrators.
 * It re-fetches after each mutation so the table reflects the server's
 * authoritative state. Errors surface as a human-readable string for the table;
 * mutations reject so the caller (the inline form) can show a per-action failure.
 *
 * When `enabled` is false (anonymous) it stays idle with an empty registry — the
 * list RPC would be `unauthenticated` server-side, so it never issues it.
 */

import { useCallback, useEffect, useState } from "react";

import type { InstrumentDef, InstrumentInput } from "../data/contract";
import type { CelnetTransport } from "../data/transport";

/** Narrow an unknown thrown value to a display string. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : "unexpected error";
}

/** The Reference Data workspace data API. */
export interface ReferenceDataApi {
  /** The current instrument registry (empty until the first load resolves). */
  instruments: InstrumentDef[];
  /** True while a load (or refetch) is in flight. */
  isLoading: boolean;
  /** The last load error as a display string, or `null`. */
  error: string | null;
  /** Re-load the registry from the server. */
  refetch: () => Promise<void>;
  /** Create a definition (blank id ⇒ server mints from name); resolves or rejects. */
  createInstrument: (input: InstrumentInput) => Promise<InstrumentDef>;
  /** Replace the identified definition; resolves or rejects. */
  updateInstrument: (input: InstrumentInput) => Promise<InstrumentDef>;
  /** Delete a definition by id; resolves to whether it existed. */
  deleteInstrument: (id: string) => Promise<boolean>;
}

export function useReferenceData(
  transport: CelnetTransport,
  enabled: boolean,
): ReferenceDataApi {
  const [instruments, setInstruments] = useState<InstrumentDef[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refetch = useCallback(async (): Promise<void> => {
    if (!enabled) {
      setInstruments([]);
      setError(null);
      return;
    }
    setIsLoading(true);
    try {
      const next = await transport.listInstruments();
      setInstruments(next);
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

  const createInstrument = useCallback(
    async (input: InstrumentInput): Promise<InstrumentDef> => {
      const created = await transport.createInstrument(input);
      await refetch();
      return created;
    },
    [transport, refetch],
  );

  const updateInstrument = useCallback(
    async (input: InstrumentInput): Promise<InstrumentDef> => {
      const updated = await transport.updateInstrument(input);
      await refetch();
      return updated;
    },
    [transport, refetch],
  );

  const deleteInstrument = useCallback(
    async (id: string): Promise<boolean> => {
      const removed = await transport.deleteInstrument(id);
      await refetch();
      return removed;
    },
    [transport, refetch],
  );

  return {
    instruments,
    isLoading,
    error,
    refetch,
    createInstrument,
    updateInstrument,
    deleteInstrument,
  };
}
