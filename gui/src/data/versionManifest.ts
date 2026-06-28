/**
 * versionManifest — release-detection seam. The running bundle bakes in its own
 * build identity (`__CELNET_BUILD_*`, stamped by vite.config.ts); the deploy emits
 * the SAME identity as a never-cached `/version.json` beside index.html. This
 * module polls that manifest and reports when the deployed identity has moved past
 * the one this page is running, so the trader can reload onto the fresh bundle.
 *
 * Pure comparison logic is split out (parseReleaseManifest / isNewerRelease) and
 * unit-tested; {@link useVersionWatch} is the thin React polling shell.
 */

import { useEffect, useRef, useState } from "react";
import type { CelnetTransport } from "./transport";

export interface ReleaseManifest {
  /** Git short SHA of the build (or `vX.Y.Z` off a git tree). */
  readonly hash: string;
  /** UTC ISO-8601 build timestamp. */
  readonly buildTime: string;
}

/** This running bundle's own build identity, baked in by Vite `define`. */
export const RUNNING_RELEASE: ReleaseManifest = {
  hash: __CELNET_BUILD_HASH__,
  buildTime: __CELNET_BUILD_TIME__,
};

/** Path of the never-cached manifest emitted beside index.html. */
export const VERSION_MANIFEST_URL = "/version.json";

/** Background poll cadence while the tab is open (ms). Deploys are infrequent. */
export const VERSION_POLL_INTERVAL_MS = 5 * 60_000;

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
 * True when `served` is a different, newer build than `running`. A differing git
 * hash is the primary signal; an identical hash with a strictly later ISO build
 * time (a rebuild of the same commit) also counts. ISO-8601 strings order
 * chronologically under lexicographic compare, so `>` is a valid recency test.
 */
export function isNewerRelease(running: ReleaseManifest, served: ReleaseManifest): boolean {
  if (served.hash !== running.hash) return true;
  return served.buildTime > running.buildTime;
}

export interface VersionWatch {
  /**
   * The newest served manifest seen that is strictly newer than the running
   * bundle, or `null` while none has been observed. Latches forward: the running
   * code can only get staler, so once set it only ever advances to an even newer
   * build — it never clears on its own.
   */
  readonly available: ReleaseManifest | null;
}

/**
 * Poll `/version.json` and report when a newer release than the running bundle is
 * live. Checks on mount, on a fixed interval, when the tab regains visibility, and
 * whenever the live transport (re)connects — a blue-green cutover that swaps the
 * bundle usually bounces the socket too, making reconnect the earliest signal a
 * new release just landed. Fetch/parse hiccups stay quiet and retry next trigger.
 *
 * Transports with no remote (the in-app `?mock` source, which exposes no
 * `onConnectionState`) simply skip the reconnect trigger; interval + visibility
 * polling still run against the statically-served manifest.
 */
export function useVersionWatch(
  transport: CelnetTransport,
  running: ReleaseManifest = RUNNING_RELEASE,
): VersionWatch {
  const [available, setAvailable] = useState<ReleaseManifest | null>(null);
  // Hold current values in refs so the checker closure (built once per transport)
  // always compares against the latest state without re-subscribing every render.
  const runningRef = useRef(running);
  runningRef.current = running;
  const availableRef = useRef<ReleaseManifest | null>(null);

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
      if (!isNewerRelease(runningRef.current, served)) return;
      const prev = availableRef.current;
      // Only advance to a strictly newer manifest than one already surfaced.
      if (prev !== null && !isNewerRelease(prev, served)) return;
      availableRef.current = served;
      setAvailable(served);
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
  }, [transport]);

  return { available };
}
