/**
 * useTableUiState — a tiny module-level store for a table's transient UI state
 * (sort, filter/search text, active lens, expanded rows, scroll offset) so it
 * SURVIVES the workspace unmounting on a tab switch and is restored on return.
 *
 * Same root cause as {@link useCachedResource}: the Shell mounts workspaces
 * conditionally, so a table's local `useState` (sort key, search query, scroll
 * position) is destroyed when the trader switches tabs and reset to its initial
 * value on return. Routing that state through this store keeps the trader's view
 * exactly where they left it.
 *
 * Keyed by a stable `tableId`. The value is a plain object; `patch` merges a partial
 * update immutably and notifies every mounted consumer of the same id.
 */

import { useCallback, useRef, useSyncExternalStore } from "react";

/** The module-level UI-state store — survives component unmount. */
const store = new Map<string, unknown>();
/** Per-id subscriber sets driving `useSyncExternalStore`. */
const listeners = new Map<string, Set<() => void>>();

function emit(tableId: string): void {
  const set = listeners.get(tableId);
  if (set) for (const l of set) l();
}

/**
 * TEST-ONLY: clear the module-level UI-state store so persisted sort/filter/lens do
 * not leak between test cases (the store intentionally survives component unmount).
 */
export function __resetTableUiStore(): void {
  store.clear();
  listeners.clear();
}

/**
 * Persist a table's UI state across unmount.
 *
 * @param tableId a stable id for this table (e.g. `"fi-deals-blotter"`)
 * @param initial the initial state (used ONLY on the first ever mount for this id)
 * @returns `[state, patch]` — `patch` merges a partial update
 */
export function useTableUiState<S extends object>(
  tableId: string,
  initial: S,
): [S, (patch: Partial<S>) => void] {
  // Capture the initial once so `getSnapshot` returns a STABLE reference when the
  // store has no entry yet (useSyncExternalStore requires a cached snapshot — a
  // fresh `initial` object each render would loop).
  const initialRef = useRef(initial);

  const subscribe = useCallback(
    (cb: () => void) => {
      let set = listeners.get(tableId);
      if (!set) {
        set = new Set();
        listeners.set(tableId, set);
      }
      set.add(cb);
      return () => {
        set.delete(cb);
        if (set.size === 0) listeners.delete(tableId);
      };
    },
    [tableId],
  );

  const getSnapshot = useCallback(
    (): S => (store.has(tableId) ? (store.get(tableId) as S) : initialRef.current),
    [tableId],
  );
  const state = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  const patch = useCallback(
    (p: Partial<S>) => {
      const cur = store.has(tableId) ? (store.get(tableId) as S) : initialRef.current;
      store.set(tableId, { ...cur, ...p });
      emit(tableId);
    },
    [tableId],
  );

  return [state, patch];
}
