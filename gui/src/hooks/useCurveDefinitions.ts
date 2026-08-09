/**
 * useCurveDefinitions — the data hook backing the Curves multi-curve manager
 * (server commit 38bcff9a). A thin layer over the transport's curve-definition
 * surface: it loads every persisted {@link CurveDefinition} when `enabled` (any
 * authenticated session — `list_curve_definitions` is open to all signed-in
 * users), and exposes the mutating operations (create / update / delete) which the
 * server gates to the Refdata·FixedIncome capability. It re-fetches after each
 * mutation so the dashboard reflects the server's authoritative state (crucially
 * the server-maintained `primary` flag, which a create/update can re-home).
 *
 * Errors surface as a human-readable string for the dashboard; mutations REJECT so
 * the caller (the definition editor, the delete control) can render a per-action
 * failure — including the server's `already_exists` / `not_found` /
 * `failed_precondition` / `invalid_argument` guidance. When `enabled` is false
 * (anonymous) it stays idle with an empty registry — the list RPC would be
 * `unauthenticated` server-side, so it is never issued.
 */

import { useCallback, useEffect, useState } from "react";

import type { CurveDefinition } from "../data/contract";
import type { CelnetTransport } from "../data/transport";

/** Narrow an unknown thrown value to a display string. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : "unexpected error";
}

/** The Curves manager data API. */
export interface CurveDefinitionsApi {
  /** The current curve-definition registry (empty until the first load resolves). */
  definitions: CurveDefinition[];
  /** True while a load (or refetch) is in flight. */
  isLoading: boolean;
  /** The last load error as a display string, or `null`. */
  error: string | null;
  /** Re-load the registry from the server. */
  refetch: () => Promise<void>;
  /** Persist a new curve; resolves to the server's canonical stored record or rejects. */
  createCurve: (definition: CurveDefinition) => Promise<CurveDefinition>;
  /** Replace the definition identified by `curveId` (immutable slug); resolves or rejects. */
  updateCurve: (
    curveId: string,
    definition: CurveDefinition,
  ) => Promise<CurveDefinition>;
  /** Delete a curve by id; resolves or rejects (primary-with-siblings ⇒ precondition). */
  deleteCurve: (curveId: string) => Promise<void>;
}

export function useCurveDefinitions(
  transport: CelnetTransport,
  enabled: boolean,
): CurveDefinitionsApi {
  const [definitions, setDefinitions] = useState<CurveDefinition[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refetch = useCallback(async (): Promise<void> => {
    if (!enabled) {
      setDefinitions([]);
      setError(null);
      return;
    }
    setIsLoading(true);
    try {
      const next = await transport.listCurveDefinitions();
      setDefinitions(next);
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

  const createCurve = useCallback(
    async (definition: CurveDefinition): Promise<CurveDefinition> => {
      const created = await transport.createCurveDefinition(definition);
      await refetch();
      return created;
    },
    [transport, refetch],
  );

  const updateCurve = useCallback(
    async (
      curveId: string,
      definition: CurveDefinition,
    ): Promise<CurveDefinition> => {
      const updated = await transport.updateCurveDefinition(curveId, definition);
      await refetch();
      return updated;
    },
    [transport, refetch],
  );

  const deleteCurve = useCallback(
    async (curveId: string): Promise<void> => {
      await transport.deleteCurveDefinition(curveId);
      await refetch();
    },
    [transport, refetch],
  );

  return {
    definitions,
    isLoading,
    error,
    refetch,
    createCurve,
    updateCurve,
    deleteCurve,
  };
}
