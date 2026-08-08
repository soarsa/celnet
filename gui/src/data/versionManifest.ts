/**
 * versionManifest — release-detection + cache-busting auto-refresh seam. The
 * running bundle bakes in its own build identity (`__CELNET_BUILD_*`, stamped by
 * vite.config.ts); the deploy emits the SAME identity as a never-cached
 * `/version.json` beside index.html. This module polls that manifest and, when the
 * deployed build has moved past the one this page is running, drives a reliable
 * cache-busting reload onto the fresh bundle — so a trader never sits on a stale
 * build thinking "the deploy did nothing".
 *
 * `hash` is NOT a reliable discriminator: the packaged deploy artifact is built
 * off a git tree, so `buildHash()` falls back to `vX.Y.Z` (currently always
 * `v0.0.0`) and every release carries the SAME hash. The real per-release
 * discriminator is `buildTime` — the UTC wall-clock at build, computed ONCE in
 * vite.config.ts and shared by BOTH sides of the seam. So {@link isNewerRelease}
 * keys strictly on `buildTime`.
 *
 * Pure comparison logic ({@link parseReleaseManifest} / {@link isNewerRelease}) and
 * the reload-orchestration primitives ({@link clearClientCaches} /
 * {@link resetAndReloadTo} + the session loop-guard) are split out and unit-tested;
 * {@link useVersionWatch} is the thin React polling shell and {@link useUpdatePending}
 * is a read-only view of the same single latch for the Settings surface (no second
 * poller).
 */

import { useEffect, useSyncExternalStore } from "react";
import type { CelnetTransport } from "./transport";

export interface ReleaseManifest {
  /**
   * Build identity tag — the git short SHA, or `vX.Y.Z` off a git tree. In the
   * packaged deploy this is a constant (`v0.0.0`) across releases, so it is NOT
   * used for recency comparison — see {@link isNewerRelease}. Retained for display.
   */
  readonly hash: string;
  /** UTC ISO-8601 build timestamp — the authoritative per-release discriminator. */
  readonly buildTime: string;
}

/** This running bundle's own build identity, baked in by Vite `define`. */
export const RUNNING_RELEASE: ReleaseManifest = {
  hash: __CELNET_BUILD_HASH__,
  buildTime: __CELNET_BUILD_TIME__,
};

/** Path of the never-cached manifest emitted beside index.html. */
export const VERSION_MANIFEST_URL = "/version.json";

/**
 * Background poll cadence while the tab is open (ms). Frequent enough that a trader
 * who leaves a tab open lands on a new deploy within a minute, cheap enough to be
 * invisible (one no-store GET of a ~60-byte JSON).
 */
export const VERSION_POLL_INTERVAL_MS = 45_000;

/**
 * sessionStorage key holding the exact `buildTime` we last performed a cache-busting
 * reload FOR, in this tab session. This is the loop-guard: we reload at most once per
 * distinct detected build, so a pathological case (the freshly-served bundle STILL
 * reporting an older baked `buildTime` than `/version.json`) can never spin the page.
 */
export const RELOAD_GUARD_KEY = "celnet:reloaded-for-build";

/**
 * Validate an untrusted `/version.json` payload into a {@link ReleaseManifest},
 * or `null` if it is not a well-formed manifest. Never trusts the wire shape.
 */
export function parseReleaseManifest(raw: unknown): ReleaseManifest | null {
  if (typeof raw !== "object" || raw === null) return null;
  const obj = raw as Record<string, unknown>;
  const { hash, buildTime } = obj;
  if (typeof hash !== "string" || hash.length === 0) return null;
  if (typeof buildTime !== "string" || buildTime.length === 0) return null;
  return { hash, buildTime };
}

/**
 * True when `served` is a strictly newer build than `running`. Keyed ON `buildTime`
 * ONLY — the deploy stamps a constant `hash` (`v0.0.0`) across releases, so `hash`
 * carries no recency signal and comparing it would false-positive forever. Both
 * sides' `buildTime` come from the SAME value computed once in vite.config.ts, so
 * an unchanged deploy compares equal (no action) and a rebuild always stamps a later
 * time. ISO-8601 strings order chronologically under lexicographic compare, so `>`
 * is a valid recency test.
 */
export function isNewerRelease(running: ReleaseManifest, served: ReleaseManifest): boolean {
  return served.buildTime > running.buildTime;
}

// ── Shared release latch ────────────────────────────────────────────────────
// The single {@link useVersionWatch} instance publishes the newest served-newer
// manifest here; any surface (the App banner, the Settings footer) subscribes via
// useSyncExternalStore. One source of truth, one poller.

let releaseLatch: ReleaseManifest | null = null;
const latchListeners = new Set<() => void>();

function subscribeLatch(onChange: () => void): () => void {
  latchListeners.add(onChange);
  return () => latchListeners.delete(onChange);
}

function getLatch(): ReleaseManifest | null {
  return releaseLatch;
}

/** Publish a newer manifest onto the shared latch, notifying all subscribers. */
function publishLatch(next: ReleaseManifest): void {
  releaseLatch = next;
  for (const l of latchListeners) l();
}

/**
 * Clear the shared release latch. The latch is module-global (it mirrors the ONE
 * deploy-watcher for the whole app), so tests that exercise the watcher reset it
 * between cases; production never needs to clear it (the running code only ever gets
 * staler, so the latch only advances).
 */
export function resetReleaseLatch(): void {
  releaseLatch = null;
  for (const l of latchListeners) l();
}

export interface VersionWatch {
  /**
   * The newest served manifest seen that is strictly newer than the running bundle,
   * or `null` while none has been observed. Latches forward: the running code can
   * only get staler, so once set it only advances to an even newer build.
   */
  readonly available: ReleaseManifest | null;
}

/**
 * Poll `/version.json` and latch when a newer release than the running bundle is
 * live. Checks on mount, on a fixed interval, when the tab regains visibility, and
 * whenever the live transport (re)connects — a blue-green cutover that swaps the
 * bundle usually bounces the socket too, making reconnect the earliest signal a new
 * release just landed. Fetch/parse hiccups stay quiet and retry next trigger.
 *
 * The fetch is always `cache: "no-store"` so the poll itself is never served from
 * the HTTP cache — the whole point is to see the freshly-deployed manifest, not a
 * cached copy of the old one.
 *
 * Transports with no remote (the in-app `?mock` source, which exposes no
 * `onConnectionState`) simply skip the reconnect trigger; interval + visibility
 * polling still run against the statically-served manifest.
 */
export function useVersionWatch(
  transport: CelnetTransport,
  running: ReleaseManifest = RUNNING_RELEASE,
): VersionWatch {
  const available = useSyncExternalStore(subscribeLatch, getLatch, getLatch);

  useEffect(() => {
    let cancelled = false;

    const check = async (): Promise<void> => {
      let served: ReleaseManifest | null;
      try {
        const res = await fetch(VERSION_MANIFEST_URL, { cache: "no-store" });
        if (!res.ok) return;
        served = parseReleaseManifest(await res.json());
      } catch {
        return; // network/parse hiccup — stay quiet, retry on the next trigger
      }
      if (cancelled || served === null) return;
      if (!isNewerRelease(running, served)) return;
      // Only advance to a build strictly newer than one already latched.
      const prev = releaseLatch;
      if (prev !== null && !isNewerRelease(prev, served)) return;
      publishLatch(served);
    };

    void check();
    const interval = setInterval(() => void check(), VERSION_POLL_INTERVAL_MS);

    const onVisibility = (): void => {
      if (document.visibilityState === "visible") void check();
    };
    document.addEventListener("visibilitychange", onVisibility);

    const disposeConn = transport.onConnectionState?.((open) => {
      if (open) void check();
    });

    return () => {
      cancelled = true;
      clearInterval(interval);
      document.removeEventListener("visibilitychange", onVisibility);
      disposeConn?.();
    };
    // `running` is the module-constant RUNNING_RELEASE in production (stable
    // identity), so this effect re-subscribes only when the transport changes.
  }, [transport, running]);

  return { available };
}

/**
 * Read-only view of the shared release latch (the newest served-newer build), for
 * surfaces that want to show "up to date" vs "update pending" WITHOUT starting a
 * second poller. Backed by the same latch {@link useVersionWatch} publishes to.
 */
export function useUpdatePending(): ReleaseManifest | null {
  return useSyncExternalStore(subscribeLatch, getLatch, getLatch);
}

// ── Cache-busting reload ─────────────────────────────────────────────────────

/**
 * Best-effort clear of every Cache Storage entry and unregistration of any service
 * worker, so a reload cannot be satisfied from a stale client-side cache. There is
 * no service worker today, but this is defensive: if one is ever introduced (or was
 * left behind by a past build), it must not pin the trader to an old bundle. Never
 * throws — each step is independently guarded so one failing does not skip the next.
 */
export async function clearClientCaches(): Promise<void> {
  try {
    if (typeof caches !== "undefined") {
      const keys = await caches.keys();
      await Promise.all(keys.map((k) => caches.delete(k)));
    }
  } catch {
    /* CacheStorage absent/blocked — nothing to clear */
  }
  try {
    const sw =
      typeof navigator !== "undefined" ? navigator.serviceWorker : undefined;
    if (sw) {
      const regs = await sw.getRegistrations();
      await Promise.all(regs.map((r) => r.unregister()));
    }
  } catch {
    /* ServiceWorker API absent/blocked — nothing to unregister */
  }
}

/** True if a cache-busting reload was already performed for `buildTime` this tab session. */
export function alreadyReloadedFor(buildTime: string, store: Storage = sessionStorage): boolean {
  try {
    return store.getItem(RELOAD_GUARD_KEY) === buildTime;
  } catch {
    return false; // storage disabled — never claim we've reloaded, but see markReloadedFor
  }
}

/** Record that we performed a cache-busting reload for `buildTime` this tab session. */
export function markReloadedFor(buildTime: string, store: Storage = sessionStorage): void {
  try {
    store.setItem(RELOAD_GUARD_KEY, buildTime);
  } catch {
    /* storage disabled/full — best-effort; the reload still happens */
  }
}

/** Injectable seams for {@link resetAndReloadTo}, so tests drive it without a real reload. */
export interface ResetAndReloadDeps {
  /** Perform the hard reload. Default: `window.location.reload()`. */
  readonly reload: () => void;
  /** Clear client caches / service workers. Default: {@link clearClientCaches}. */
  readonly clearCaches: () => Promise<void>;
  /** The loop-guard store. Default: `sessionStorage`. */
  readonly store: Storage;
}

/**
 * Cache-busting reload onto `target`. Idempotent per target within a tab session:
 * the FIRST call for a given `buildTime` marks the loop-guard, clears client caches
 * + unregisters service workers, then hard-reloads. Any subsequent call for the SAME
 * `buildTime` is a no-op — this is what makes a reload-loop impossible: even if the
 * page comes back still reporting an older baked build than `/version.json`, the
 * guard has already recorded that target and refuses to reload again.
 *
 * The content-hashed JS/CSS (`assets/index-<hash>.js`) change filename per build so
 * they are refetched by name; `index.html` is served `no-store` so the document
 * itself is refetched — clearing caches + reload is belt-and-braces on top of that.
 *
 * Returns `true` iff a reload was triggered.
 */
export async function resetAndReloadTo(
  target: ReleaseManifest,
  deps: Partial<ResetAndReloadDeps> = {},
): Promise<boolean> {
  const store = deps.store ?? sessionStorage;
  const reload = deps.reload ?? ((): void => window.location.reload());
  const clearCaches = deps.clearCaches ?? clearClientCaches;

  if (alreadyReloadedFor(target.buildTime, store)) return false;
  markReloadedFor(target.buildTime, store);
  await clearCaches();
  reload();
  return true;
}
