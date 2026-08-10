/**
 * useCachedResource — a tiny stale-while-revalidate cache for the trader GUI's
 * read-only table workspaces.
 *
 * WHY: the Shell mounts each workspace CONDITIONALLY (`{tab === "x" && <Ws/>}`),
 * so switching tabs UNMOUNTS the previous workspace. A workspace that fetches into
 * local `useState` (starting EMPTY) therefore loses its rows on every tab switch —
 * the table blanks, refetches, and repopulates (the "tables clear and reload every
 * time you switch tabs" defect). This hook moves the fetched data into a
 * MODULE-LEVEL store that survives unmount, exposed through `useSyncExternalStore`
 * so every mounted consumer of the same key re-renders when the data changes.
 *
 * Behaviour (SWR):
 *   • cache HIT  → the cached `data` is returned immediately (`isLoading=false`); if
 *     it is stale (older than `ttlMs`) or the `deps` changed, a revalidation runs in
 *     the BACKGROUND (`isValidating=true`) WITHOUT clearing the visible `data`.
 *   • cache MISS → `isLoading=true`, the fetcher runs, the store is populated.
 *   • concurrent consumers of a key share ONE in-flight promise (dedupe); a resolved
 *     fetch whose in-flight token was superseded is ignored.
 *   • a fetch error never throws: `error` is exposed and the last good `data` is
 *     kept on screen.
 *   • `refresh()` forces a background revalidation (keeps the current data visible)
 *     — the seam the streaming ticks call instead of a raw refetch that would clear.
 *   • `primeCachedResource(key, data)` writes a value imperatively — the seam a
 *     PUSH subscription (which delivers the payload itself) uses to keep the cache
 *     line hot without a redundant fetch.
 *
 * The cache line is identified ENTIRELY by `key` — callers compose the key from the
 * RPC name + serialized args + scope, so a scope/args change is a different cache
 * line (its own data), never a clobber of the previous one.
 *
 * This is intentionally minimal (no external dependency): the app ships no query
 * library, and the read tables need exactly stale-while-revalidate + dedupe, not a
 * full cache manager.
 */

import { useCallback, useEffect, useRef, useSyncExternalStore } from "react";

/** How long a cache line stays "fresh" before a mount/return revalidates it (ms). */
const DEFAULT_TTL_MS = 5_000;

/** One cache line's immutable snapshot (replaced wholesale on every change so
 * `useSyncExternalStore` sees a new reference and re-renders). */
interface Entry {
  /** The last successfully-fetched (or primed) value, or `undefined` before any. */
  readonly data: unknown;
  /** The last fetch error (kept alongside the last good `data`), or `undefined`. */
  readonly error: unknown;
  /** When `data`/`error` was last written (ms epoch); 0 ⇒ never fetched. */
  readonly updatedAt: number;
  /** The serialized `deps` the current `data` corresponds to. */
  readonly depsKey: string;
  /** True while a fetch (first-load OR background revalidation) is in flight. */
  readonly isValidating: boolean;
}

const EMPTY_ENTRY: Entry = {
  data: undefined,
  error: undefined,
  updatedAt: 0,
  depsKey: "",
  isValidating: false,
};

/** The module-level store — the data that SURVIVES component unmount. */
const store = new Map<string, Entry>();
/** In-flight promises per key (dedupe), kept OUT of the snapshot object. */
const inflight = new Map<string, Promise<void>>();
/** Per-key identity token for the CURRENT fetch, so a superseded resolution is ignored. */
const tokens = new Map<string, object>();
/** Per-key subscriber sets driving `useSyncExternalStore`. */
const listeners = new Map<string, Set<() => void>>();

function emit(key: string): void {
  const set = listeners.get(key);
  if (set) for (const l of set) l();
}

/** Replace a key's entry with `{...cur, ...patch}` (immutable) and notify. */
function setEntry(key: string, patch: Partial<Entry>): void {
  const cur = store.get(key) ?? EMPTY_ENTRY;
  store.set(key, { ...cur, ...patch });
  emit(key);
}

/** Serialize a value to a stable string (bigint-safe). */
function stableStringify(v: unknown): string {
  return JSON.stringify(v, (_k, val) => (typeof val === "bigint" ? `${val}n` : val));
}

/**
 * Serialize an argument/scope value into a cache-KEY fragment (bigint-safe). Callers
 * compose a cache line's key from the RPC name + these fragments (e.g. the
 * entitlement principal / scope) so a scope or args change is a distinct cache line.
 */
export function cacheKeyPart(v: unknown): string {
  return v === undefined ? "∅" : stableStringify(v);
}

/** Serialize a `deps` array to a stable string (bigint-safe). */
function serializeDeps(deps: readonly unknown[]): string {
  return stableStringify(deps);
}

/**
 * Run (or join) a fetch for `key`. Dedupes against any in-flight fetch for the same
 * key. On success writes `data` + `depsKey`; on failure records `error` but KEEPS
 * the last good `data`. A resolution whose in-flight token was superseded is ignored.
 */
function runFetch(
  key: string,
  fetcher: () => Promise<unknown>,
  depsKey: string,
): Promise<void> {
  const existing = inflight.get(key);
  if (existing) return existing;

  const token = {};
  tokens.set(key, token);
  setEntry(key, { isValidating: true });
  const p = (async () => {
    try {
      const data = await fetcher();
      if (tokens.get(key) !== token) return; // superseded
      setEntry(key, {
        data,
        error: undefined,
        updatedAt: Date.now(),
        depsKey,
        isValidating: false,
      });
    } catch (error) {
      if (tokens.get(key) !== token) return; // superseded
      // Keep the last good `data`; surface the error; stamp updatedAt so we do not
      // tight-loop retry (the next revalidation waits for the ttl / an explicit refresh).
      setEntry(key, { error, updatedAt: Date.now(), depsKey, isValidating: false });
    } finally {
      if (tokens.get(key) === token) inflight.delete(key);
    }
  })();
  inflight.set(key, p);
  return p;
}

/**
 * Write a value into the cache imperatively (a PUSH subscription that already holds
 * the payload). Keeps the line hot so a remount reads it instantly without a fetch.
 */
export function primeCachedResource<T>(key: string, data: T): void {
  setEntry(key, { data, error: undefined, updatedAt: Date.now() });
}

/**
 * TEST-ONLY: clear the entire module-level cache (data, in-flight, tokens,
 * listeners). Called from the global test `afterEach` so the store — which by design
 * survives component unmount — does not leak state between test cases.
 */
export function __resetCachedResourceStore(): void {
  store.clear();
  inflight.clear();
  tokens.clear();
  listeners.clear();
}

/** Options controlling a cached resource's revalidation. */
export interface CachedResourceOptions {
  /** Extra reactive inputs; a change revalidates in the background (data kept). */
  readonly deps?: readonly unknown[];
  /** Freshness window (ms); older ⇒ a mount/return revalidates. Default 5s. */
  readonly ttlMs?: number;
  /** When false the resource does not fetch (e.g. signed-out); `isLoading=false`. */
  readonly enabled?: boolean;
}

/** The stale-while-revalidate view of one cache line. */
export interface CachedResource<T> {
  /** The cached value (kept across unmount); `undefined` only before the first load. */
  readonly data: T | undefined;
  /** True on the FIRST load only (no data yet). Never true on a background revalidate. */
  readonly isLoading: boolean;
  /** True while a background revalidation is in flight (data already on screen). */
  readonly isValidating: boolean;
  /** The last fetch error, alongside the last good `data`. */
  readonly error: unknown;
  /** Force a background revalidation (keeps current data visible). */
  readonly refresh: () => void;
}

/**
 * Subscribe to the cache line `key`, fetching via `fetcher` on a miss / when stale.
 *
 * @param key    the cache line identity (RPC name + serialized args + scope)
 * @param fetcher the async loader (read through a ref, so an inline arrow is fine)
 * @param opts    deps / ttl / enabled
 */
export function useCachedResource<T>(
  key: string,
  fetcher: () => Promise<T>,
  opts: CachedResourceOptions = {},
): CachedResource<T> {
  const { deps = [], ttlMs = DEFAULT_TTL_MS, enabled = true } = opts;
  const depsKey = serializeDeps(deps);

  // Read the fetcher through a ref so a fresh inline arrow each render does not
  // re-trigger the effect (the effect depends on key/depsKey/enabled, not identity).
  const fetcherRef = useRef(fetcher);
  fetcherRef.current = fetcher;

  const subscribe = useCallback(
    (cb: () => void) => {
      let set = listeners.get(key);
      if (!set) {
        set = new Set();
        listeners.set(key, set);
      }
      set.add(cb);
      return () => {
        set.delete(cb);
        if (set.size === 0) listeners.delete(key);
      };
    },
    [key],
  );
  const getSnapshot = useCallback(() => store.get(key), [key]);
  const entry = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  // On mount / key / deps / enabled change: fetch on a miss, or revalidate when the
  // line is stale (ttl exceeded) or the deps moved. A fresh line is left untouched.
  useEffect(() => {
    if (!enabled) return;
    const e = store.get(key);
    const now = Date.now();
    const fresh =
      e !== undefined &&
      e.updatedAt > 0 &&
      e.depsKey === depsKey &&
      now - e.updatedAt <= ttlMs;
    if (fresh) return;
    void runFetch(key, () => fetcherRef.current(), depsKey);
  }, [key, depsKey, enabled, ttlMs]);

  const refresh = useCallback(() => {
    void runFetch(key, () => fetcherRef.current(), depsKey);
  }, [key, depsKey]);

  const data = entry?.data as T | undefined;
  const error = entry?.error;
  const isValidating = entry?.isValidating ?? false;
  // First-load only: no data AND no error yet, and the resource is enabled.
  const isLoading = enabled && data === undefined && error === undefined;

  return { data, isLoading, isValidating, error, refresh };
}
