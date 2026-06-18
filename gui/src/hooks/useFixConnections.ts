/**
 * Owns the managed inbound FIX-acceptor list for the Connections workspace.
 *
 * A thin data hook over the transport's `FixAdminService` surface: it loads the
 * list, exposes the mutating operations (create / update / enable-disable /
 * delete), and re-fetches after each mutation so the table always reflects the
 * server's authoritative state (including the server-minted id and the live
 * `running`/`boundAddr` status). Errors are surfaced as a human-readable string
 * for the caller to render; mutations reject so a caller (the wizard) can show a
 * per-action failure inline.
 */

import { useCallback, useEffect, useState } from "react";

import type { FixConnection, FixConnectionSpec } from "../data/contract";
import type { CelnetTransport } from "../data/transport";

/** Narrow an unknown thrown value to a display string. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : "unexpected error";
}

/** The Connections-workspace data API. */
export interface FixConnectionsApi {
  /** The current managed connections (empty until the first load resolves). */
  connections: FixConnection[];
  /** True while the initial load (or a refetch) is in flight. */
  isLoading: boolean;
  /** The last load error as a display string, or `null` when healthy. */
  error: string | null;
  /** Re-load the list from the server. */
  refetch: () => Promise<void>;
  /** Define a new connection; resolves to the created descriptor or rejects. */
  create: (spec: FixConnectionSpec) => Promise<FixConnection>;
  /** Replace an existing connection's definition. */
  update: (id: string, spec: FixConnectionSpec) => Promise<FixConnection>;
  /** Enable or disable a connection (bind/stop its acceptor). */
  setEnabled: (id: string, enabled: boolean) => Promise<FixConnection>;
  /** Delete a connection (stops its acceptor). */
  remove: (id: string) => Promise<void>;
}

export function useFixConnections(transport: CelnetTransport): FixConnectionsApi {
  const [connections, setConnections] = useState<FixConnection[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refetch = useCallback(async (): Promise<void> => {
    setIsLoading(true);
    try {
      const list = await transport.listFixConnections();
      setConnections(list);
      setError(null);
    } catch (e: unknown) {
      setError(messageOf(e));
    } finally {
      setIsLoading(false);
    }
  }, [transport]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      setIsLoading(true);
      try {
        const list = await transport.listFixConnections();
        if (!cancelled) {
          setConnections(list);
          setError(null);
        }
      } catch (e: unknown) {
        if (!cancelled) setError(messageOf(e));
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [transport]);

  const create = useCallback(
    async (spec: FixConnectionSpec): Promise<FixConnection> => {
      const created = await transport.createFixConnection(spec);
      await refetch();
      return created;
    },
    [transport, refetch],
  );

  const update = useCallback(
    async (id: string, spec: FixConnectionSpec): Promise<FixConnection> => {
      const updated = await transport.updateFixConnection(id, spec);
      await refetch();
      return updated;
    },
    [transport, refetch],
  );

  const setEnabled = useCallback(
    async (id: string, enabled: boolean): Promise<FixConnection> => {
      const next = await transport.setFixConnectionEnabled(id, enabled);
      await refetch();
      return next;
    },
    [transport, refetch],
  );

  const remove = useCallback(
    async (id: string): Promise<void> => {
      await transport.deleteFixConnection(id);
      await refetch();
    },
    [transport, refetch],
  );

  return { connections, isLoading, error, refetch, create, update, setEnabled, remove };
}
